//! `verum doc-render` subcommand — auto-paper generator surface.
//!
//! Walks every `.vr` file in the project (using the same
//! `audit::discover_vr_files` + `audit::parse_file_for_audit`
//! helpers as the verify-ladder integration), projects each
//! @theorem / @lemma / @corollary / @axiom to a typed
//! [`DocItem`](verum_verification::doc_render::DocItem), and feeds
//! the resulting [`DocCorpus`] to
//! [`DefaultDocRenderer`](verum_verification::doc_render::DefaultDocRenderer).
//!
//! ## Why a sibling to `doc.rs`
//!
//! The pre-existing `commands/doc.rs` is a Rust-style API-doc
//! generator that walks `///` comments on functions/types — useful
//! but a different concern than the auto-paper pipeline: the
//! auto-paper output renders the *formal statement + proof* of every
//! theorem with cross-references, citation graph, and reproducibility
//! envelope (closure hash).  Two separate generators with two
//! separate use cases.
//!
//! ## Subcommands
//!
//!   * `verum doc-render render [--format md|latex|html] [--out <PATH>] [--public]`
//!     Render the entire corpus.
//!   * `verum doc-render graph [--format dot|json] [--public]`
//!     Citation-graph export.
//!   * `verum doc-render check-refs [--format plain|json] [--public]`
//!     Broken-cross-ref audit (CI-friendly; non-zero exit on broken refs).

use crate::error::{CliError, Result};
use std::path::PathBuf;
use verum_ast::decl::ItemKind;
use verum_common::Text;
use verum_verification::doc_render::{
    build_citation_allowlist, collect_proof_citations, DefaultDocRenderer, DocCorpus,
    DocItem, DocItemKind, DocRenderer, RenderFormat,
};

use super::audit::{discover_vr_files, parse_file_for_audit};

/// Walk every `.vr` file under the manifest dir and project each
/// declaration to a typed `DocItem`.
///
/// Citation resolution is two-pass: pass 1 builds the corpus's
/// name allowlist (every Theorem / Lemma / Corollary / Axiom);
/// pass 2 walks each proof body's AST via
/// [`collect_proof_citations`] and filters references against that
/// allowlist (#92 hardening).  Replaces the previous
/// `Debug`-formatted-string heuristic that produced both false
/// positives (struct-debug noise from `format!("{:?}", body)`) and
/// false negatives (citations whose names didn't end in `_lemma` /
/// `_thm`).
fn collect_corpus(only_public: bool) -> Result<DocCorpus> {
    let manifest_dir = crate::config::Manifest::find_manifest_dir()?;
    let vr_files = discover_vr_files(&manifest_dir);

    // Parse every file once.
    let mut modules: Vec<(verum_ast::Module, PathBuf)> = Vec::new();
    for abs_path in &vr_files {
        let rel_path = abs_path
            .strip_prefix(&manifest_dir)
            .unwrap_or(abs_path)
            .to_path_buf();
        if let Ok(m) = parse_file_for_audit(abs_path) {
            modules.push((m, rel_path));
        }
    }

    // Pass 1: project items with empty citations; remember each
    // item's location so pass 2 can re-resolve.
    let mut pending: Vec<(DocItem, usize, usize)> = Vec::new();
    for (mi, (module, rel_path)) in modules.iter().enumerate() {
        for (ii, item) in module.items.iter().enumerate() {
            if let Some(doc) = project_item(item, rel_path, only_public) {
                pending.push((doc, mi, ii));
            }
        }
    }

    // Build the corpus-wide allowlist from every projected name.
    let docs_only: Vec<DocItem> = pending.iter().map(|(d, _, _)| d.clone()).collect();
    let allowlist = build_citation_allowlist(&docs_only);

    // Pass 2: re-resolve citations using the AST visitor.
    let mut items: Vec<DocItem> = Vec::with_capacity(pending.len());
    for (mut doc, mi, ii) in pending {
        let module = &modules[mi].0;
        let item = &module.items[ii];
        if let Some(proof) = proof_body_of(item) {
            let cites = collect_proof_citations(proof, &allowlist);
            // Don't cite yourself — the proof body sometimes contains
            // recursive references (`induction n; case S => apply
            // foo`); the self-cite is structural noise.
            let self_name = doc.name.as_str().to_string();
            doc.citations = cites
                .into_iter()
                .filter(|c| c.as_str() != self_name)
                .collect();
        }
        items.push(doc);
    }
    items.sort_by(|a, b| a.name.as_str().cmp(b.name.as_str()));
    Ok(DocCorpus::new(items))
}

/// Borrow the proof body of a theorem-shaped item, if any.
fn proof_body_of(item: &verum_ast::decl::Item) -> Option<&verum_ast::decl::ProofBody> {
    use verum_common::Maybe;
    let proof = match &item.kind {
        ItemKind::Theorem(t) => &t.proof,
        ItemKind::Lemma(t) => &t.proof,
        ItemKind::Corollary(t) => &t.proof,
        _ => return None,
    };
    match proof {
        Maybe::Some(b) => Some(b),
        Maybe::None => None,
    }
}

/// Project one AST item to a `DocItem` if it's a renderable kind.
fn project_item(
    item: &verum_ast::decl::Item,
    rel_path: &PathBuf,
    only_public: bool,
) -> Option<DocItem> {
    use verum_ast::attr::FrameworkAttr;
    use verum_common::Maybe;

    // Theorem-shaped declarations carry requires/ensures/proof; axioms
    // are postulated propositions only.  Project to a uniform tuple.
    enum Shape<'a> {
        Theoremish {
            requires: &'a verum_common::List<verum_ast::Expr>,
            ensures: &'a verum_common::List<verum_ast::Expr>,
            proof: Option<&'a verum_ast::decl::ProofBody>,
        },
        Axiomatic,
    }

    let (kind, name, attrs, shape, is_public) = match &item.kind {
        ItemKind::Theorem(t) => (
            DocItemKind::Theorem,
            t.name.name.clone(),
            &t.attributes,
            Shape::Theoremish {
                requires: &t.requires,
                ensures: &t.ensures,
                proof: t.proof.as_ref(),
            },
            t.visibility.is_public(),
        ),
        ItemKind::Lemma(t) => (
            DocItemKind::Lemma,
            t.name.name.clone(),
            &t.attributes,
            Shape::Theoremish {
                requires: &t.requires,
                ensures: &t.ensures,
                proof: t.proof.as_ref(),
            },
            t.visibility.is_public(),
        ),
        ItemKind::Corollary(t) => (
            DocItemKind::Corollary,
            t.name.name.clone(),
            &t.attributes,
            Shape::Theoremish {
                requires: &t.requires,
                ensures: &t.ensures,
                proof: t.proof.as_ref(),
            },
            t.visibility.is_public(),
        ),
        ItemKind::Axiom(a) => (
            DocItemKind::Axiom,
            a.name.name.clone(),
            &a.attributes,
            Shape::Axiomatic,
            a.visibility.is_public(),
        ),
        _ => return None,
    };

    if only_public && !is_public {
        return None;
    }

    let signature = format!("{} {}(...)", kind.name(), name.as_str());
    let (requires_rendered, ensures_rendered, proof_steps): (Vec<Text>, Vec<Text>, Vec<Text>) =
        match shape {
            Shape::Theoremish {
                requires,
                ensures,
                proof,
            } => {
                let req: Vec<Text> = requires
                    .iter()
                    .map(|e| Text::from(format!("{:?}", e)))
                    .collect();
                let ens: Vec<Text> = ensures
                    .iter()
                    .map(|e| Text::from(format!("{:?}", e)))
                    .collect();
                let steps = match proof {
                    Some(body) => render_proof_steps(body),
                    None => Vec::new(),
                };
                (req, ens, steps)
            }
            Shape::Axiomatic => (Vec::new(), Vec::new(), Vec::new()),
        };
    // Citations are resolved in `collect_corpus` pass 2 against the
    // corpus-wide allowlist; here we leave them empty and let the
    // outer loop fill them in.
    let citations: Vec<Text> = Vec::new();
    let mut framework_markers: Vec<(Text, Text)> = Vec::new();
    for attr in attrs.iter() {
        if !attr.is_named("framework") {
            continue;
        }
        if let Maybe::Some(fw) = FrameworkAttr::from_attribute(attr) {
            framework_markers.push((fw.name.clone(), fw.citation.clone()));
        }
    }

    Some(DocItem {
        kind,
        name,
        docstring: Text::from(""),
        signature: Text::from(signature),
        requires: requires_rendered,
        ensures: ensures_rendered,
        proof_steps,
        citations,
        framework_markers,
        closure_hash: None,
        source_file: Text::from(rel_path.display().to_string()),
        source_line: 1,
    })
}

/// Best-effort proof-body → tactic-step list extraction.  V0 ships a
/// shallow projection that captures named tactic invocations by
/// walking the `Debug`-rendered body and pulling out one line per
/// surface step.  V1 will replace this with a proper
/// `ProofBody → Vec<TacticStep>` projection.
fn render_proof_steps(body: &verum_ast::decl::ProofBody) -> Vec<Text> {
    let raw = format!("{:?}", body);
    raw.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty()
                || line.starts_with('}')
                || line.starts_with('{')
                || line.starts_with("ProofBody")
            {
                return None;
            }
            Some(Text::from(line.to_string()))
        })
        .take(50)
        .collect()
}


// =============================================================================
// validate format helpers
// =============================================================================

fn parse_render_format(s: &str) -> Result<RenderFormat> {
    RenderFormat::from_name(s).ok_or_else(|| {
        CliError::InvalidArgument(format!(
            "--format must be one of markdown / md / latex / tex / html, got '{}'",
            s
        ))
    })
}

fn validate_text_format(s: &str) -> Result<()> {
    if s != "plain" && s != "json" {
        return Err(CliError::InvalidArgument(format!(
            "--format must be 'plain' or 'json', got '{}'",
            s
        )));
    }
    Ok(())
}

// =============================================================================
// run_render
// =============================================================================

pub fn run_render(
    format: &str,
    out: Option<&PathBuf>,
    only_public: bool,
) -> Result<()> {
    let fmt = parse_render_format(format)?;
    let corpus = collect_corpus(only_public)?;
    let renderer = DefaultDocRenderer::new();
    let rendered = renderer
        .render_corpus(&corpus, fmt)
        .map_err(|e| CliError::VerificationFailed(format!("render: {}", e)))?;
    match out {
        Some(path) => {
            std::fs::write(path, rendered.as_str()).map_err(|e| {
                CliError::VerificationFailed(format!(
                    "write {}: {}",
                    path.display(),
                    e
                ))
            })?;
        }
        None => {
            println!("{}", rendered.as_str());
        }
    }
    Ok(())
}

// =============================================================================
// run_graph — citation-graph export
// =============================================================================

pub fn run_graph(format: &str, only_public: bool) -> Result<()> {
    let corpus = collect_corpus(only_public)?;
    match format {
        "dot" => {
            println!("{}", corpus.to_dot().as_str());
            Ok(())
        }
        "json" => {
            let g = corpus.citation_graph();
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!(
                "  \"item_count\": {},\n",
                corpus.items.len()
            ));
            out.push_str("  \"edges\": [\n");
            let mut edges: Vec<(String, String)> = Vec::new();
            for (k, v) in &g {
                for c in v {
                    edges.push((k.as_str().to_string(), c.as_str().to_string()));
                }
            }
            for (i, (from, to)) in edges.iter().enumerate() {
                out.push_str(&format!(
                    "    {{ \"from\": \"{}\", \"to\": \"{}\" }}{}\n",
                    json_escape(from),
                    json_escape(to),
                    if i + 1 < edges.len() { "," } else { "" }
                ));
            }
            out.push_str("  ]\n}");
            println!("{}", out);
            Ok(())
        }
        other => Err(CliError::InvalidArgument(format!(
            "graph --format must be 'dot' or 'json', got '{}'",
            other
        ))),
    }
}

// =============================================================================
// run_check_refs — broken-cross-ref audit
// =============================================================================

pub fn run_check_refs(format: &str, only_public: bool) -> Result<()> {
    validate_text_format(format)?;
    let corpus = collect_corpus(only_public)?;
    let broken = corpus.validate_cross_refs();
    match format {
        "plain" => {
            if broken.is_empty() {
                println!(
                    "✓ All {} item(s) have resolved cross-references.",
                    corpus.items.len()
                );
            } else {
                println!(
                    "✗ Found {} broken cross-reference(s):",
                    broken.len()
                );
                for b in &broken {
                    println!(
                        "  {} → {}",
                        b.citing_item.as_str(),
                        b.broken_target.as_str()
                    );
                }
            }
        }
        "json" => {
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!(
                "  \"item_count\": {},\n",
                corpus.items.len()
            ));
            out.push_str(&format!("  \"broken_count\": {},\n", broken.len()));
            out.push_str("  \"broken\": [\n");
            for (i, b) in broken.iter().enumerate() {
                out.push_str(&format!(
                    "    {{ \"citing_item\": \"{}\", \"broken_target\": \"{}\" }}{}\n",
                    json_escape(b.citing_item.as_str()),
                    json_escape(b.broken_target.as_str()),
                    if i + 1 < broken.len() { "," } else { "" }
                ));
            }
            out.push_str("  ]\n}");
            println!("{}", out);
        }
        _ => unreachable!(),
    }
    if !broken.is_empty() {
        return Err(CliError::VerificationFailed(format!(
            "{} broken cross-reference(s) — fix `apply X;` / `\\ref{{X}}` targets",
            broken.len()
        )));
    }
    Ok(())
}

// =============================================================================
// helpers
// =============================================================================

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- format parsing -----

    #[test]
    fn parse_render_format_accepts_aliases() {
        for (s, expected) in [
            ("markdown", RenderFormat::Markdown),
            ("md", RenderFormat::Markdown),
            ("latex", RenderFormat::Latex),
            ("tex", RenderFormat::Latex),
            ("html", RenderFormat::Html),
        ] {
            assert_eq!(parse_render_format(s).unwrap(), expected);
        }
    }

    #[test]
    fn parse_render_format_rejects_unknown() {
        assert!(matches!(
            parse_render_format("yaml"),
            Err(CliError::InvalidArgument(_))
        ));
    }

    #[test]
    fn validate_text_format_round_trip() {
        assert!(validate_text_format("plain").is_ok());
        assert!(validate_text_format("json").is_ok());
        assert!(matches!(
            validate_text_format("yaml"),
            Err(CliError::InvalidArgument(_))
        ));
    }

    // ----- citations: AST-based path (collect_proof_citations) -----
    //
    // The earlier test set covered a `citations_from_text` text-
    // search heuristic that used a free-form "find tokens
    // ending in `_lemma`" walk. That heuristic was replaced by
    // the AST-based `collect_proof_citations` walk
    // (verum_compiler::derives) which is more principled — it
    // operates on the proof's parsed AST rather than guessing
    // from source text. The heuristic was removed but the
    // tests were left orphaned, breaking the lib-test build.
    // Removed here; the production path is now exercised by
    // the doc-corpus integration tests in `tests/`.

    // ----- json_escape -----

    #[test]
    fn json_escape_handles_quotes_and_newlines() {
        assert_eq!(json_escape("a\"b\nc"), "a\\\"b\\nc");
    }

    // ----- format validation contracts -----

    #[test]
    fn parse_render_format_recognises_html_explicitly() {
        assert_eq!(
            parse_render_format("html").unwrap(),
            RenderFormat::Html
        );
    }
}
