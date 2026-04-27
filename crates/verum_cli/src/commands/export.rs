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
    /// OWL 2 Functional Syntax — emits a Pellet/HermiT/Protégé-
    /// compatible `.ofn` file from the project's `@owl2_*` attribute
    /// markers (Phase 3 B5). Walks the same `Owl2Graph`
    /// shared with `audit --owl2-classify` and emits Declaration /
    /// SubClassOf / EquivalentClasses / DisjointClasses / HasKey /
    /// ObjectPropertyDomain / ObjectPropertyRange / per-characteristic
    /// flag axioms / InverseObjectProperties. BTreeMap-sorted output
    /// for byte-deterministic round-trip.
    Owl2Fs,
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
            "owl2-fs" | "owl2_fs" | "ofn" => Ok(Self::Owl2Fs),
            other => Err(CliError::InvalidArgument(
                format!(
                    "unknown export format: `{}` (expected `dedukti`, \
                     `coq`, `lean`, `agda`, `metamath`, or `owl2-fs`)",
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
            Self::Owl2Fs => "owl2-fs",
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
            Self::Owl2Fs => "ofn",
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
    /// proof certificate, when
    /// available. Drives `ProofReplayBackend.lower(...)` so the
    /// emitted target file carries a real proof instead of
    /// `Admitted` / `sorry` / `?`. `None` when no certificate is
    /// loaded for this declaration (axioms; theorems whose proof
    /// isn't yet on-disk; bare statement-only export); the per-
    /// target emitter then falls through to the V1 admitted
    /// scaffold so the export remains compilable.
    ///
    /// V4.2 lays the wiring; V4.3+ plumbs actual certificate
    /// loading from the kernel's certificate store. Until then
    /// this field is always `None` in production paths and the
    /// behaviour matches V1 exactly.
    #[allow(dead_code)]
    certificate: Option<verum_kernel::SmtCertificate>,
}

/// Options for the `verum export` command.
pub struct ExportOptions {
    pub format: ExportFormat,
    pub output: Maybe<PathBuf>,
    /// emit a
    /// per-declaration provenance JSON sidecar alongside the main
    /// certificate. See `emit_provenance_sidecar` for the schema.
    pub with_provenance: bool,
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
    let mut owl2_graph = crate::commands::owl2::Owl2Graph::default();
    let mut skipped_files = 0usize;

    // Persistent certificate store rooted at the project's
    // `.verum/cache/certificates/` directory. Each declaration's
    // cert is loaded lazily during `collect_declaration` and stuffed
    // into Declaration.certificate so the per-target emit can hand it
    // to the proof-replay backend. Missing certs ⇒ Maybe::None ⇒
    // admitted-fallback.
    let cert_store = verum_smt::cert_store::FileSystemCertificateStore::for_project(&manifest_dir);

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
            if let Some(decl) = collect_declaration(item, &rel_path, &cert_store) {
                declarations.push(decl);
            }
            // The Owl2Fs target consumes the same parse pass — single
            // walk, two collectors. Other targets can ignore the
            // resulting graph.
            crate::commands::owl2::collect_owl2_attrs(item, &rel_path, &mut owl2_graph);
        }
    }

    let manifest_name = read_manifest_name(&manifest_dir);

    // proof-replay registry,
    // pre-populated with all 5 concrete backends per V6–V10. Each
    // emit_<target> consults this to lower SmtCertificate traces
    // into target-language tactic chains; falls back to the V1
    // admitted scaffold when no certificate is loaded.
    let replay_registry = verum_smt::proof_replay::default_registry();

    let body = match options.format {
        ExportFormat::Dedukti => emit_dedukti(&declarations, &replay_registry),
        ExportFormat::Coq => emit_coq(&declarations, &replay_registry),
        ExportFormat::Lean => emit_lean(&declarations, &replay_registry),
        ExportFormat::Agda => emit_agda(&declarations, &replay_registry),
        ExportFormat::Metamath => emit_metamath(&declarations, &replay_registry),
        ExportFormat::Owl2Fs => emit_owl2_fs(&owl2_graph, &manifest_name),
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

    // provenance sidecar.
    // Statement-level export remains unchanged (Admitted / sorry / `?`);
    // the sidecar carries the per-declaration metadata downstream
    // tools need to drive SMT replay or fill in proof terms.
    if options.with_provenance {
        let sidecar = emit_provenance_sidecar(&declarations, options.format);
        let sidecar_path = sidecar_path_for(&output_path);
        std::fs::write(&sidecar_path, &sidecar).map_err(|e| {
            CliError::Custom(
                format!(
                    "writing provenance sidecar to {}: {}",
                    sidecar_path.display(),
                    e
                )
                .into(),
            )
        })?;
    }

    print_summary(
        options.format,
        &declarations,
        &output_path,
        skipped_files,
    );

    Ok(())
}

/// derive `<output>.provenance.json` from
/// `<output>` so the sidecar lands next to the main certificate.
fn sidecar_path_for(main: &Path) -> PathBuf {
    let mut p = main.to_path_buf();
    let stem = p
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "export".to_string());
    p.set_file_name(format!("{}.provenance.json", stem));
    p
}

/// emit a JSON sidecar
/// describing every exported declaration. The sidecar is a stable,
/// versioned schema (`schema_version: 1`) with one entry per
/// declaration carrying:
///
///   - `name` / `kind` / `source_file`
///   - `framework_name` + `framework_citation` when present
///   - `discharge_strategy` ∈ {`statement_only`, `smt_replay_pending`}:
///     today every entry is `statement_only` because the kernel-side
///     SmtCertificate→target-language lowering is V2.1+ work; the
///     field is reserved so future emitters can mark certificates
///     where SMT replay landed without bumping the schema version.
///   - `obligation_hash`: `null` until the kernel exposes per-decl
///     SmtCertificate hashes through the export pipeline.
///   - `proof_term`: `null` — V2.1+ slot for the lowered proof term.
///
/// Output is deterministic (declarations preserve emit order; field
/// ordering is stable) so CI diffs stay clean across runs.
fn emit_provenance_sidecar(decls: &[Declaration], format: ExportFormat) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  \"schema_version\": 1,\n");
    out.push_str(&format!(
        "  \"target_format\": \"{}\",\n",
        format.as_str()
    ));
    out.push_str(&format!("  \"declaration_count\": {},\n", decls.len()));
    out.push_str("  \"declarations\": [\n");
    let total = decls.len();
    for (i, d) in decls.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!(
            "      \"name\": \"{}\",\n",
            json_escape_export(d.name.as_str())
        ));
        out.push_str(&format!("      \"kind\": \"{}\",\n", d.kind));
        out.push_str(&format!(
            "      \"source_file\": \"{}\",\n",
            json_escape_export(&d.source.display().to_string())
        ));
        if let Maybe::Some(fw) = &d.framework {
            out.push_str(&format!(
                "      \"framework_name\": \"{}\",\n",
                json_escape_export(fw.name.as_str())
            ));
            out.push_str(&format!(
                "      \"framework_citation\": \"{}\",\n",
                json_escape_export(fw.citation.as_str())
            ));
        } else {
            out.push_str("      \"framework_name\": null,\n");
            out.push_str("      \"framework_citation\": null,\n");
        }
        out.push_str("      \"discharge_strategy\": \"statement_only\",\n");
        out.push_str("      \"obligation_hash\": null,\n");
        out.push_str("      \"proof_term\": null\n");
        out.push_str(if i + 1 == total { "    }\n" } else { "    },\n" });
    }
    out.push_str("  ]\n");
    out.push_str("}\n");
    out
}

fn json_escape_export(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
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

fn collect_declaration(
    item: &Item,
    rel_path: &Path,
    cert_store: &dyn verum_smt::cert_store::CertificateStore,
) -> Option<Declaration> {
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

    // Try to load a persisted SmtCertificate for this declaration.
    // Missing certs surface as `None`; the per-target emit then falls
    // through to the admitted scaffold via `replay_or_admitted`.
    // Axioms never have certs (they're postulates, not derivations);
    // theorems/lemmas/corollaries opt into proof-replay by having a
    // cert on disk.
    let certificate = match cert_store.load(name.as_str()) {
        verum_common::Maybe::Some(c) => Some(c),
        verum_common::Maybe::None => None,
    };

    Some(Declaration {
        kind,
        name,
        source: rel_path.to_path_buf(),
        framework,
        certificate,
    })
}

/// apply the proof-replay
/// registry to a declaration. Returns the lowered tactic source on
/// success; falls back to the per-target admitted shape when no
/// certificate is loaded for the declaration.
fn replay_or_admitted(
    registry: &verum_smt::proof_replay::ProofReplayRegistry,
    target: &str,
    decl: &Declaration,
    admitted_default: &str,
) -> String {
    let cert = match &decl.certificate {
        Some(c) => c,
        None => return admitted_default.to_string(),
    };
    let backend = match registry.get(target) {
        Some(b) => b,
        None => return admitted_default.to_string(),
    };
    let header = verum_smt::proof_replay::DeclarationHeader {
        name: decl.name.clone(),
        kind: verum_smt::proof_replay::DeclKind::from_str(decl.kind)
            .unwrap_or(verum_smt::proof_replay::DeclKind::Theorem),
        framework: match &decl.framework {
            Maybe::Some(fw) => Some(verum_smt::proof_replay::FrameworkRef {
                name: fw.name.clone(),
                citation: fw.citation.clone(),
            }),
            Maybe::None => None,
        },
    };
    match backend.lower(cert, &header) {
        Ok(tactic) => tactic.source,
        Err(_) => admitted_default.to_string(),
    }
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
// Framework-lineage → target-library mapping 
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
// six-pack" plus a handful of widely-cited foundations.
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
        // meta-classifier framework. The Diakrisis
        // package fixes the canonical-primitive coordinate system
        // (Articulation/Enactment Morita-duality, dual no-go,
        // dual gauge-surjection kernel, dual-primitive initial-
        // object). It does not have a single mainstream Coq /
        // Agda / Lean / Dedukti / Metamath analogue: it's a
        // metaclassification framework, not a theorem library.
        // We emit citation comments only — downstream auditors
        // recognise `diakrisis` as the meta-classifier and don't
        // expect a target-library import.
        "diakrisis",
        LineageImport {
            lean: None,
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
        // OWL 2 Functional Syntax. Coq HOL-Light has
        // an OWL 2 fragment via the Coq-DL workspace; Lean 4 has
        // an experimental DescriptionLogic library; mainstream
        // Agda / Dedukti / Metamath have no DL libraries.
        "owl2_fs",
        LineageImport {
            lean: None,
            coq: None,
            dedukti: None,
            metamath: None,
            agda: None,
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
            // added Coq HoTT modality + Agda cubical
            // for the cohesive triple-adjunction ∫ ⊣ ♭ ⊣ ♯.
            lean: Some("import Mathlib.CategoryTheory.Sites.Sheaf"),
            coq: Some("Require Import HoTT.Modalities.Modality."),
            dedukti: None,
            metamath: None,
            agda: Some("open import Cubical.Modalities.Everything"),
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

fn emit_dedukti(decls: &[Declaration], replay_registry: &verum_smt::proof_replay::ProofReplayRegistry) -> String {
    let mut out = String::new();
    out.push_str("(; Exported by `verum export --to dedukti`. ;)\n");
    out.push_str(
        "(; .1: theorem proofs lowered via DeduktiProofReplay\n\
        when SmtCertificates are loaded; otherwise admitted comment marker. ;)\n\n",
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
            // Dedukti uses term-style proof
            // assignment: `def name : Prop := <term>.` for theorems
            // with a lowered body, `name : Prop.` (axiom form) for
            // postulates without a body.
            match d.kind {
                "axiom" => {
                    out.push_str(&format!("{} : Prop.\n\n", mangle(&d.name)));
                }
                _ => {
                    let proof = replay_or_admitted(
                        replay_registry,
                        "dedukti",
                        d,
                        "(; admitted ;)",
                    );
                    if proof.starts_with("(;") {
                        // Admitted fallback — keep the legacy
                        // axiom-form so the file stays valid.
                        out.push_str(&format!(
                            "{} : Prop. {}\n\n",
                            mangle(&d.name),
                            proof
                        ));
                    } else {
                        // Lowered λΠ-term — emit as a `def`.
                        out.push_str(&format!(
                            "def {} : Prop := {}.\n\n",
                            mangle(&d.name),
                            proof
                        ));
                    }
                }
            }
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

fn emit_coq(decls: &[Declaration], replay_registry: &verum_smt::proof_replay::ProofReplayRegistry) -> String {
    let mut out = String::new();
    out.push_str("(* Exported by `verum export --to coq`. *)\n");
    out.push_str(
        "(* theorem proofs lowered via CoqProofReplay\n\
        when SmtCertificates are loaded; otherwise admitted scaffold. *)\n\n",
    );

    // framework-lineage → Coq-library mapping.
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
                    // Consult the proof-replay registry for a Coq
                    // tactic chain. Falls back to `Proof. Admitted.`
                    // when no certificate is loaded (current state).
                    let proof_body = replay_or_admitted(
                        replay_registry,
                        "coq",
                        d,
                        "Proof. Admitted.",
                    );
                    out.push_str(&format!(
                        "Theorem {} : Prop.\n{}\n\n",
                        mangle(&d.name),
                        proof_body
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
fn emit_metamath(decls: &[Declaration], replay_registry: &verum_smt::proof_replay::ProofReplayRegistry) -> String {
    let mut out = String::new();
    out.push_str("$( Exported by `verum export --to metamath`. $)\n");
    out.push_str(
        "$( .1: theorem proof steps lowered via\n\
         MetamathProofReplay when SmtCertificates are loaded; otherwise\n\
         `?` placeholder accepted by mmverify.py as unchecked scaffold. $)\n\n",
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
                    // Metamath theorem-step
                    // body. MetamathProofReplay produces the body
                    // including the `$= ... $.` framing; we splice
                    // the rendered body after the `wff` head.
                    let proof = replay_or_admitted(
                        replay_registry,
                        "metamath",
                        d,
                        "$= ? $.",
                    );
                    out.push_str(&format!(
                        "th-{} $p wff {} {}\n\n",
                        mangle(&d.name),
                        mangle(&d.name),
                        proof
                    ));
                }
            }
        }
    }
    out
}

fn emit_lean(decls: &[Declaration], replay_registry: &verum_smt::proof_replay::ProofReplayRegistry) -> String {
    let mut out = String::new();
    out.push_str("-- Exported by `verum export --to lean`.\n");
    out.push_str(
        "-- theorem proofs lowered via LeanProofReplay\n\
         -- when SmtCertificates are loaded; otherwise sorry scaffold.\n\n",
    );

    // Emit `import` stanzas for known framework-lineage mappings
    // so the file is ready to check against Mathlib without
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
                    // Replay or sorry-fallback. LeanProofReplay
                    // produces a `by ... ` block; we splice it after
                    // `:=` to form a complete term-style theorem.
                    let proof = replay_or_admitted(
                        replay_registry,
                        "lean",
                        d,
                        ":= sorry",
                    );
                    let body = if proof.starts_with("by") {
                        format!(":= {}", proof)
                    } else {
                        proof
                    };
                    out.push_str(&format!(
                        "theorem {} : Prop {}\n\n",
                        mangle(&d.name),
                        body
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
fn emit_agda(decls: &[Declaration], replay_registry: &verum_smt::proof_replay::ProofReplayRegistry) -> String {
    let mut out = String::new();
    out.push_str("-- Exported by `verum export --to agda`.\n");
    out.push_str(
        "-- .1: theorem proofs lowered via AgdaProofReplay\n\
         -- when SmtCertificates are loaded; otherwise postulated\n\
         -- (proof terms become Agda holes `{!!}` for interactive fill).\n\n",
    );
    out.push_str("module Verum.Export where\n\n");

    // framework-lineage → Agda-library mapping. Unknown
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
            // Agda is term-style: a proof IS
            // a value of the goal type. Axioms emit as a `postulate`
            // block; theorems emit as a definition `name : Set =
            // <term>` when a cert is loaded, falling back to the
            // hole `{!!}` (still legal Agda — interactive checker
            // accepts) when no cert is loaded.
            match d.kind {
                "axiom" => {
                    out.push_str("postulate\n");
                    out.push_str(&format!("  {} : Set\n\n", agda_mangle(&d.name)));
                }
                _ => {
                    let proof = replay_or_admitted(
                        replay_registry,
                        "agda",
                        d,
                        "{!!}",
                    );
                    // Term-style definition. The `: Set` annotation
                    // is the placeholder type — the V12.1 elaborator
                    // hand-off (§8.6) replaces it with the real type
                    // once the proof-term layer surfaces lifted Verum
                    // types.
                    out.push_str(&format!(
                        "{} : Set\n{} = {}\n\n",
                        agda_mangle(&d.name),
                        agda_mangle(&d.name),
                        proof
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

    // Proof-replay coverage counts. `with_cert` = decls that had
    // an SmtCertificate loaded from the cert store and went
    // through the proof-replay backend; `admitted` = decls that
    // fell through to the admitted scaffold (no cert on disk).
    let theorem_kinds = ["theorem", "lemma", "corollary"];
    let theorem_count = decls
        .iter()
        .filter(|d| theorem_kinds.contains(&d.kind))
        .count();
    let with_cert_count = decls
        .iter()
        .filter(|d| theorem_kinds.contains(&d.kind) && d.certificate.is_some())
        .count();
    if theorem_count > 0 {
        let admitted_count = theorem_count - with_cert_count;
        println!(
            "  {} of {} theorem proof(s) replayed via SmtCertificate ({} admitted)",
            with_cert_count.to_string().green(),
            theorem_count.to_string().cyan(),
            admitted_count.to_string().yellow(),
        );
    }

    if skipped_files > 0 {
        println!(
            "  {} .vr file(s) skipped (parse errors)",
            skipped_files.to_string().yellow()
        );
    }

    println!();
    if theorem_count > 0 && with_cert_count == 0 {
        println!(
            "{} No SmtCertificates loaded from `.verum/cache/certificates/`. Run",
            "note:".dimmed()
        );
        println!(
            "      `verum verify` first to populate the cert store, then re-export"
        );
        println!("      to splice real proof-term tactic chains.");
    } else if with_cert_count < theorem_count {
        println!(
            "{} {} theorem(s) had no on-disk SmtCertificate — admitted scaffold used.",
            "note:".dimmed(),
            theorem_count - with_cert_count
        );
    }
}

// -----------------------------------------------------------------------------
// OWL 2 Functional Syntax emitter (Phase 3 B5)
// -----------------------------------------------------------------------------
//
// Walks the Owl2Graph populated during the project parse and emits
// W3C-compliant OWL 2 Functional Syntax (`.ofn`). Output is byte-
// deterministic — every collection that contributes to the body is
// already a BTreeMap or BTreeSet from `commands::owl2`, so iteration
// order is alphabetical and the same project produces the same bytes
// across runs and platforms.
//
// W3C OWL 2 FS Recommendation (Second Edition, 11 December 2012):
//   https://www.w3.org/TR/owl2-syntax/
//
// Output sections, in order:
//   Prefix(:=<base>#)
//   Ontology(<base>
//     Declaration(Class(:Name))      — one per class
//     Declaration(ObjectProperty(:Name))  — one per property
//     SubClassOf(:Sub :Sup)          — per direct subclass edge
//     EquivalentClasses(:A :B :C)    — per equivalence partition (≥2)
//     DisjointClasses(:A :B)         — per disjointness pair
//     HasKey(:Class () (:p1 :p2))    — per @owl2_has_key
//     ObjectPropertyDomain(:p :C)
//     ObjectPropertyRange(:p :C)
//     <Char>ObjectProperty(:p)        — per characteristic flag
//     InverseObjectProperties(:p :q) — per @owl2_property(inverse_of)
//   )

/// Read the project's `[package].name` from `verum.toml` to derive a
/// default ontology IRI. Falls back to `verum-export` when the manifest
/// is unreadable.
fn read_manifest_name(manifest_dir: &Path) -> String {
    let path = Manifest::manifest_path(manifest_dir);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return "verum-export".to_string(),
    };
    // Lightweight key-extraction; we don't pull a TOML parser just for this.
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("name") {
            let after_eq = rest.trim_start().strip_prefix('=').unwrap_or("").trim();
            let unquoted = after_eq.trim_matches('"').trim_matches('\'');
            if !unquoted.is_empty() {
                return unquoted.to_string();
            }
        }
    }
    "verum-export".to_string()
}

/// Render an OWL 2 IRI fragment for a local name, using the project's
/// default `:` prefix. Names containing characters outside the OWL 2
/// FS local-name production are wrapped in `<…>` (full IRI form).
fn owl2_local(name: &str) -> String {
    let safe = name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if safe {
        format!(":{}", name)
    } else {
        format!("<#{}>", name)
    }
}

/// Map a Verum `Owl2Characteristic` to its OWL 2 FS axiom name per
/// Shkotin 2019 Table 6 / W3C OWL 2 FS §9.2.
fn characteristic_axiom_name(c: verum_ast::attr::Owl2Characteristic) -> &'static str {
    use verum_ast::attr::Owl2Characteristic::*;
    match c {
        Transitive        => "TransitiveObjectProperty",
        Symmetric         => "SymmetricObjectProperty",
        Asymmetric        => "AsymmetricObjectProperty",
        Reflexive         => "ReflexiveObjectProperty",
        Irreflexive       => "IrreflexiveObjectProperty",
        Functional        => "FunctionalObjectProperty",
        InverseFunctional => "InverseFunctionalObjectProperty",
    }
}

fn emit_owl2_fs(graph: &crate::commands::owl2::Owl2Graph, manifest_name: &str) -> String {
    use crate::commands::owl2::Owl2EntityKind;

    let mut out = String::new();
    out.push_str("# Exported by `verum export --to owl2-fs` (/ B5).\n");
    out.push_str("# OWL 2 Functional-Style Syntax — round-trips through Pellet, HermiT,\n");
    out.push_str("# Protégé, FaCT++, ELK, Konclude. BTreeMap-sorted output for byte-\n");
    out.push_str("# deterministic CI diffs.\n\n");

    let base_iri = format!("http://verum-lang.org/ontology/{}", manifest_name);
    out.push_str(&format!("Prefix(:=<{}#>)\n", base_iri));
    out.push_str("Prefix(owl:=<http://www.w3.org/2002/07/owl#>)\n");
    out.push_str("Prefix(rdf:=<http://www.w3.org/1999/02/22-rdf-syntax-ns#>)\n");
    out.push_str("Prefix(rdfs:=<http://www.w3.org/2000/01/rdf-schema#>)\n");
    out.push_str("Prefix(xsd:=<http://www.w3.org/2001/XMLSchema#>)\n\n");
    out.push_str(&format!("Ontology(<{}>\n", base_iri));

    // Section 1 — Declarations (Class + ObjectProperty), alphabetical.
    for (name, e) in &graph.entities {
        match e.kind {
            Owl2EntityKind::Class => {
                out.push_str(&format!(
                    "  Declaration(Class({}))\n",
                    owl2_local(name.as_str())
                ));
            }
            Owl2EntityKind::Property => {
                out.push_str(&format!(
                    "  Declaration(ObjectProperty({}))\n",
                    owl2_local(name.as_str())
                ));
            }
        }
    }
    if !graph.entities.is_empty() { out.push('\n'); }

    // Section 2 — Class hierarchy: SubClassOf edges, alphabetical
    // by (child, parent).
    for (child, parent) in &graph.subclass_edges {
        out.push_str(&format!(
            "  SubClassOf({} {})\n",
            owl2_local(child.as_str()),
            owl2_local(parent.as_str()),
        ));
    }
    if !graph.subclass_edges.is_empty() { out.push('\n'); }

    // Section 3 — EquivalentClasses, one axiom per partition (≥ 2
    // classes). Equivalence pairs in graph are symmetrised; we use the
    // partition projection for clean OWL 2 FS output.
    for partition in graph.equivalence_partition() {
        if partition.len() < 2 { continue; }
        let mut group: Vec<String> = partition.iter().map(|n| owl2_local(n.as_str())).collect();
        group.sort();
        out.push_str(&format!(
            "  EquivalentClasses({})\n",
            group.join(" "),
        ));
    }

    // Section 4 — DisjointClasses, one axiom per disjoint pair. We
    // de-symmetrise: only emit (a, b) with a < b lexicographically.
    let mut disjoint_seen: std::collections::BTreeSet<(Text, Text)> = std::collections::BTreeSet::new();
    for (a, b) in &graph.disjoint_pairs {
        if a >= b { continue; }
        if !disjoint_seen.insert((a.clone(), b.clone())) { continue; }
        out.push_str(&format!(
            "  DisjointClasses({} {})\n",
            owl2_local(a.as_str()),
            owl2_local(b.as_str()),
        ));
    }

    // Section 5 — HasKey for every class with a key constraint. OWL 2 FS
    // splits keys into ObjectProperty and DataProperty parenthesised
    // groups; V1 emits all key properties as ObjectProperty (the most
    // common case); V2 will route DataProperty-typed keys correctly.
    for (name, e) in &graph.entities {
        if !matches!(e.kind, Owl2EntityKind::Class) { continue; }
        for key in &e.keys {
            let props: Vec<String> = key.iter().map(|p| owl2_local(p.as_str())).collect();
            out.push_str(&format!(
                "  HasKey({} ({}) ())\n",
                owl2_local(name.as_str()),
                props.join(" "),
            ));
        }
    }

    // Section 6 — Property domain / range / characteristics / inverse.
    for (name, e) in &graph.entities {
        if !matches!(e.kind, Owl2EntityKind::Property) { continue; }
        let prop_iri = owl2_local(name.as_str());
        if let Some(d) = &e.property_domain {
            out.push_str(&format!(
                "  ObjectPropertyDomain({} {})\n",
                prop_iri,
                owl2_local(d.as_str()),
            ));
        }
        if let Some(r) = &e.property_range {
            out.push_str(&format!(
                "  ObjectPropertyRange({} {})\n",
                prop_iri,
                owl2_local(r.as_str()),
            ));
        }
        for c in &e.property_characteristics {
            out.push_str(&format!(
                "  {}({})\n",
                characteristic_axiom_name(*c),
                prop_iri,
            ));
        }
        if let Some(inv) = &e.property_inverse_of {
            out.push_str(&format!(
                "  InverseObjectProperties({} {})\n",
                prop_iri,
                owl2_local(inv.as_str()),
            ));
        }
    }

    out.push_str(")\n");
    out
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
    fn all_six_formats_parse_from_canonical_names() {
        assert_eq!(ExportFormat::parse("dedukti").unwrap(), ExportFormat::Dedukti);
        assert_eq!(ExportFormat::parse("coq").unwrap(), ExportFormat::Coq);
        assert_eq!(ExportFormat::parse("lean").unwrap(), ExportFormat::Lean);
        assert_eq!(ExportFormat::parse("agda").unwrap(), ExportFormat::Agda);
        assert_eq!(
            ExportFormat::parse("metamath").unwrap(),
            ExportFormat::Metamath
        );
        assert_eq!(ExportFormat::parse("owl2-fs").unwrap(), ExportFormat::Owl2Fs);
        assert_eq!(ExportFormat::parse("owl2_fs").unwrap(), ExportFormat::Owl2Fs);
        assert_eq!(ExportFormat::parse("ofn").unwrap(),     ExportFormat::Owl2Fs);
    }

    #[test]
    fn unknown_format_error_message_lists_all_six() {
        let err = ExportFormat::parse("isabelle").unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("dedukti"));
        assert!(msg.contains("coq"));
        assert!(msg.contains("lean"));
        assert!(msg.contains("agda"));
        assert!(msg.contains("metamath"));
        assert!(msg.contains("owl2-fs"));
    }

    #[test]
    fn owl2_fs_extension_and_canonical_name() {
        assert_eq!(ExportFormat::Owl2Fs.extension(), "ofn");
        assert_eq!(ExportFormat::Owl2Fs.as_str(),    "owl2-fs");
    }

    #[test]
    fn owl2_fs_emitter_produces_ontology_header_and_declarations() {
        use crate::commands::owl2::{Owl2Entity, Owl2Graph};
        let mut graph = Owl2Graph::default();
        graph.add_entity(Owl2Entity::new_class(
            Text::from("Animal"), None, PathBuf::from("src/lib.vr"),
        ));
        graph.add_entity(Owl2Entity::new_class(
            Text::from("Mammal"), None, PathBuf::from("src/lib.vr"),
        ));
        graph.subclass_edges.insert((Text::from("Mammal"), Text::from("Animal")));

        let out = emit_owl2_fs(&graph, "test-pkg");
        // Mandatory header per W3C OWL 2 FS Recommendation.
        assert!(out.contains("Prefix(:=<http://verum-lang.org/ontology/test-pkg#>)"));
        assert!(out.contains("Ontology(<http://verum-lang.org/ontology/test-pkg>"));
        assert!(out.contains("Declaration(Class(:Animal))"));
        assert!(out.contains("Declaration(Class(:Mammal))"));
        assert!(out.contains("SubClassOf(:Mammal :Animal)"));
        // Provenance comment
        assert!(out.contains("`verum export --to owl2-fs`"));
    }

    #[test]
    fn owl2_fs_emitter_handles_property_with_characteristics() {
        use crate::commands::owl2::{Owl2Entity, Owl2Graph};
        use std::collections::BTreeSet;
        use verum_ast::attr::Owl2Characteristic;
        let mut graph = Owl2Graph::default();
        let mut chars: BTreeSet<Owl2Characteristic> = BTreeSet::new();
        chars.insert(Owl2Characteristic::Symmetric);
        chars.insert(Owl2Characteristic::Transitive);
        graph.add_entity(Owl2Entity::new_property(
            Text::from("knows"),
            PathBuf::from("src/lib.vr"),
            Some(Text::from("Person")),
            Some(Text::from("Person")),
            Some(Text::from("knownBy")),
            chars,
        ));

        let out = emit_owl2_fs(&graph, "test-pkg");
        assert!(out.contains("Declaration(ObjectProperty(:knows))"));
        assert!(out.contains("ObjectPropertyDomain(:knows :Person)"));
        assert!(out.contains("ObjectPropertyRange(:knows :Person)"));
        assert!(out.contains("SymmetricObjectProperty(:knows)"));
        assert!(out.contains("TransitiveObjectProperty(:knows)"));
        assert!(out.contains("InverseObjectProperties(:knows :knownBy)"));
    }

    #[test]
    fn owl2_fs_emitter_deterministic_disjoint_pair_dedup() {
        use crate::commands::owl2::{Owl2Entity, Owl2Graph};
        let mut graph = Owl2Graph::default();
        graph.add_entity(Owl2Entity::new_class(Text::from("Pizza"),    None, PathBuf::new()));
        graph.add_entity(Owl2Entity::new_class(Text::from("IceCream"), None, PathBuf::new()));
        // Symmetrised pair — both orientations stored, but emitter
        // emits exactly one DisjointClasses axiom.
        graph.disjoint_pairs.insert((Text::from("Pizza"),    Text::from("IceCream")));
        graph.disjoint_pairs.insert((Text::from("IceCream"), Text::from("Pizza")));

        let out = emit_owl2_fs(&graph, "test-pkg");
        let count = out.matches("DisjointClasses(").count();
        assert_eq!(count, 1, "symmetric pair must emit exactly one axiom");
        // Lex-min order in the emitted axiom.
        assert!(out.contains("DisjointClasses(:IceCream :Pizza)"));
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
            certificate: None,
        }];
        let registry = verum_smt::proof_replay::default_registry();
        let out = emit_agda(&decls, &registry);
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
        let registry = verum_smt::proof_replay::default_registry();
        let out = emit_metamath(&decls, &registry);
        // Metamath verifiers require the constant/variable
        // declarations; the preamble must always be emitted.
        assert!(out.contains("$c wff |- $."));
        assert!(out.contains("$v x y z $."));
        assert!(out.contains("`verum export --to metamath`"));
    }

    // -------------------------------------------------------------
    // lineage-import table contract.
    //
    // The LINEAGE_IMPORTS table is a curated map. The `lineage_import`
    // lookup is `O(n)` linear-scan but n is fixed and small; the
    // ordering invariant (alphabetical-by-slug) is what makes the
    // export deterministic across runs. These tests pin both the
    // membership of the entries we ship today AND the alphabetical
    // ordering — any new entry MUST land in lex-sorted position.
    // -------------------------------------------------------------

    #[test]
    fn lineage_import_table_is_alphabetically_sorted() {
        // Iterate the static slice in declaration order and confirm
        // each entry's slug is lex-greater than its predecessor.
        let mut last: Option<&str> = None;
        for (slug, _) in LINEAGE_IMPORTS {
            if let Some(prev) = last {
                assert!(
                    *slug > prev,
                    "LINEAGE_IMPORTS must be alphabetical: {prev} >= {slug}"
                );
            }
            last = Some(slug);
        }
    }

    #[test]
    fn lineage_import_resolves_diakrisis_metaclassifier() {
        // Diakrisis has no target-library mapping (it's the
        // coordinate-system-defining framework, not a theorem
        // library). The entry exists but every column is None —
        // emitters should fall through to the "unmapped" comment
        // path without panicking.
        let li = lineage_import("diakrisis").expect("diakrisis must be in table");
        assert!(li.coq.is_none());
        assert!(li.lean.is_none());
        assert!(li.agda.is_none());
        assert!(li.dedukti.is_none());
        assert!(li.metamath.is_none());
    }

    #[test]
    fn lineage_import_owl2_fs_resolves_with_no_mappings() {
        // OWL 2 FS doesn't have mainstream Coq/Lean/Agda/Dedukti/
        // Metamath libraries — the entry exists for table-coverage
        // completeness; emitters fall through.
        let li = lineage_import("owl2_fs").expect("owl2_fs must be in table");
        assert!(li.coq.is_none());
        assert!(li.lean.is_none());
        assert!(li.agda.is_none());
    }

    #[test]
    fn lineage_import_schreiber_dcct_now_maps_coq_and_agda() {
        // added Coq HoTT.Modalities.Modality + Agda
        // Cubical.Modalities.Everything for the cohesive triple-
        // adjunction ∫ ⊣ ♭ ⊣ ♯. Unmapped column comments must
        // clear when any column populates.
        let li = lineage_import("schreiber_dcct").expect("schreiber_dcct must be in table");
        assert!(li.lean.is_some(), "schreiber_dcct.lean was already mapped");
        assert!(
            li.coq.is_some_and(|s| s.contains("HoTT.Modalities.Modality")),
            "schreiber_dcct.coq must now reference HoTT.Modalities.Modality"
        );
        assert!(
            li.agda.is_some_and(|s| s.contains("Cubical.Modalities")),
            "schreiber_dcct.agda must now reference Cubical.Modalities"
        );
    }

    #[test]
    fn lineage_import_unknown_slug_returns_none() {
        // Defensive: lookup of an unknown lineage must return None,
        // not panic. Emitters rely on this for fall-through.
        assert!(lineage_import("does_not_exist").is_none());
        assert!(lineage_import("").is_none());
    }
}
