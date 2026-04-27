//! `verum extract <file.vr> [--output <path>]` — extract executable
//! programs from constructive proofs marked with `@extract` /
//! `@extract_witness` / `@extract_contract` typed attributes
//! ().
//!
//! Walk the project's `.vr` files, collect every item carrying an
//! Extract* typed attribute, dispatch to the [`verum_smt::program_extraction`]
//! pipeline at the attribute's `ExtractTarget`, and emit a per-target
//! file at `<output>/<decl>.<ext>` (`.vr` / `.ml` / `.lean` / `.v`).
//!
//! V12 (this revision) ships the **driver scaffolding**: walks the
//! AST, projects each marked declaration into a target-language
//! scaffold via the existing `CodeGenerator`, writes the result to
//! disk. The full proof-term-to-program lowering through
//! `ProgramExtractor` requires an elaborated `ProofTerm`, which
//! lands per V12.1 once the elaborator surfaces proof terms to the
//! CLI layer; V12 emits the scaffold + extraction metadata so the
//! V12.1 hand-off is plug-and-play.

use std::fs;
use std::path::PathBuf;

use crate::config::Manifest;
use crate::error::{CliError, Result};
use crate::ui;

use verum_ast::attr::{
    ExtractAttr, ExtractContractAttr, ExtractTarget, ExtractWitnessAttr,
};
use verum_ast::decl::ItemKind;
use verum_ast::Item;
use verum_common::{Maybe, Text};

// per-target Verum AST →
// target-language lowerers. Each module exposes `lower_block` /
// `lower_expr` returning `Option<String>` (None on unsupported
// constructs → caller falls back to the V12.1 metadata comment).
mod ocaml_lower;
mod lean_lower;
mod coq_lower;

/// Options for `verum extract`.
pub struct ExtractOptions {
    /// Optional explicit input path. When `None`, all `.vr` files
    /// under the project's manifest dir are scanned.
    pub input: Option<PathBuf>,
    /// Optional output directory. Defaults to `extracted/`.
    pub output: Option<PathBuf>,
}

/// One marked declaration discovered during the walk.
#[derive(Debug, Clone)]
struct ExtractRequest {
    /// Item name (function / theorem).
    name: Text,
    /// Source `.vr` file the request came from.
    source: PathBuf,
    /// Extraction kind — full program, witness-only, or contract-only.
    kind: ExtractKind,
    /// Target language.
    target: ExtractTarget,
    /// captured source text
    /// of the declaration's body (function body / theorem proof
    /// body) extracted via the AST node's `Span`. `None` for
    /// declarations without a body (axioms; signatures).
    ///
    /// When present and the target is Verum, the emitter splices
    /// this body verbatim — the Verum extracted file is then
    /// re-checkable by `verum check`. For other targets V12.1
    /// is staging: the body is recorded in a metadata comment
    /// for V12.2's per-target lowerers (OCaml / Lean / Coq).
    body_source: Option<String>,
    /// declared signature
    /// (parameter list + return type) captured verbatim from
    /// source so the extracted Verum file preserves the
    /// declaration's interface. `None` when the source-text
    /// snippet wasn't available (parse-only flow).
    signature_source: Option<String>,
    /// visibility keyword
    /// to prepend when the captured signature doesn't already
    /// start with one (`public` / `internal` / etc.). The AST
    /// `Span` for `Function`/`Theorem`/`Lemma`/`Corollary`
    /// covers from the keyword (`fn` / `theorem` / …) — not
    /// the visibility modifier — so we capture visibility
    /// separately and prepend on emit.
    visibility_keyword: Option<&'static str>,
    /// captured body AST for
    /// per-target lowering. Stored as a [`BodyAst`] discriminant
    /// because functions can carry either a `Block` (`{ stmts; tail }`)
    /// or a single-`Expr` body (`= expr;`). Theorems carry the
    /// `proposition` Expr — captured as `BodyAst::Expr`. `None`
    /// for declarations without an extractable body (axioms /
    /// signatures-only fns).
    body_ast: Option<BodyAst>,
    /// Native binding from `@extract(realize="fn_name")`. When
    /// `Some`, the emitted scaffold is a thin wrapper that
    /// delegates to the named function instead of synthesising a
    /// body from the proof term — used to bind verified scaffolds
    /// to runtime intrinsics or hand-written native primitives.
    realize: Option<String>,
}

/// body shape carried into the V12.2 lowerers.
/// Mirrors [`verum_ast::decl::FunctionBody`] but unifies theorem
/// proposition-Expr and function single-Expr bodies into one variant.
#[derive(Debug, Clone)]
enum BodyAst {
    /// Block body `{ stmt; …; tail-expr }`.
    Block(verum_ast::expr::Block),
    /// Single-expression body (function-expression form, theorem
    /// proposition, lemma proposition).
    Expr(verum_ast::expr::Expr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExtractKind {
    Program,
    Witness,
    Contract,
}

impl ExtractKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Program => "@extract",
            Self::Witness => "@extract_witness",
            Self::Contract => "@extract_contract",
        }
    }
}

/// Entry point for `verum extract [<input>] [--output <path>]`.
pub fn run(options: ExtractOptions) -> Result<()> {
    ui::step("Walking @extract / @extract_witness / @extract_contract markers");

    let manifest_dir = Manifest::find_manifest_dir()?;
    let vr_files = match &options.input {
        Some(p) => vec![p.clone()],
        None => discover_vr_files(&manifest_dir),
    };

    if vr_files.is_empty() {
        ui::warn("no .vr files to scan");
        return Ok(());
    }

    let mut requests: Vec<ExtractRequest> = Vec::new();
    let mut skipped = 0usize;

    for abs_path in &vr_files {
        let rel_path = abs_path
            .strip_prefix(&manifest_dir)
            .unwrap_or(abs_path)
            .to_path_buf();
        // read source text alongside parse so
        // `collect_extract_requests` can capture body/signature
        // source via AST node spans.
        let source_text = match std::fs::read_to_string(abs_path) {
            Ok(s) => s,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let module = match parse_file(abs_path) {
            Ok(m) => m,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        for item in &module.items {
            collect_extract_requests(item, &rel_path, &source_text, &mut requests);
        }
    }

    if requests.is_empty() {
        ui::info("no @extract markers found in scanned files");
        return Ok(());
    }

    let output_dir = options
        .output
        .unwrap_or_else(|| manifest_dir.join("extracted"));
    fs::create_dir_all(&output_dir).map_err(|e| {
        CliError::Custom(
            format!("creating output directory {}: {}", output_dir.display(), e)
                .into(),
        )
    })?;

    let mut emitted = 0usize;
    for req in &requests {
        let body = emit_scaffold(req);
        let file_name = format!(
            "{}.{}",
            req.name.as_str(),
            extension_for(req.target)
        );
        let path = output_dir.join(&file_name);
        fs::write(&path, &body).map_err(|e| {
            CliError::Custom(
                format!("writing {}: {}", path.display(), e).into(),
            )
        })?;
        emitted += 1;
    }

    println!(
        "Extracted {} declaration(s) ({} skipped) to {}",
        emitted,
        skipped,
        output_dir.display()
    );
    Ok(())
}

fn discover_vr_files(root: &std::path::Path) -> Vec<PathBuf> {
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

fn parse_file(path: &std::path::Path) -> std::result::Result<verum_ast::Module, String> {
    use verum_compiler::pipeline::CompilationPipeline;
    use verum_compiler::session::Session;
    use verum_compiler::CompilerOptions;
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

/// Walk an item's attributes and emit one [`ExtractRequest`] per
/// recognised Extract* attribute. Items can carry multiple
/// extractions (e.g. `@extract(verum)` + `@extract_witness(coq)`)
/// and each becomes a separate request.
fn collect_extract_requests(
    item: &Item,
    rel_path: &std::path::Path,
    source_text: &str,
    out: &mut Vec<ExtractRequest>,
) {
    use verum_ast::span::Spanned;
    let item_start = item.span.start;
    let visibility_keyword: Option<&'static str> = match &item.kind {
        ItemKind::Function(decl) => visibility_keyword_for(&decl.visibility),
        ItemKind::Theorem(decl)
        | ItemKind::Lemma(decl)
        | ItemKind::Corollary(decl) => visibility_keyword_for(&decl.visibility),
        _ => None,
    };
    let (item_name, decl_attrs, body_source, signature_source, body_ast): (
        Text,
        &verum_common::List<verum_ast::attr::Attribute>,
        Option<String>,
        Option<String>,
        Option<BodyAst>,
    ) = match &item.kind {
        ItemKind::Function(decl) => {
            let body_src = match &decl.body {
                Maybe::Some(verum_ast::decl::FunctionBody::Block(b)) => {
                    span_slice(source_text, b.span())
                }
                Maybe::Some(verum_ast::decl::FunctionBody::Expr(e)) => {
                    span_slice(source_text, e.span)
                }
                Maybe::None => None,
            };
            let sig_src = match &decl.body {
                Maybe::Some(verum_ast::decl::FunctionBody::Block(b)) => {
                    span_slice_range(source_text, item_start, b.span().start)
                }
                Maybe::Some(verum_ast::decl::FunctionBody::Expr(e)) => {
                    span_slice_range(source_text, item_start, e.span.start)
                }
                Maybe::None => span_slice_range(source_text, item_start, item.span.end),
            };
            // V12.2: capture body AST for per-target lowering.
            let body_ast = match &decl.body {
                Maybe::Some(verum_ast::decl::FunctionBody::Block(b)) => {
                    Some(BodyAst::Block(b.clone()))
                }
                Maybe::Some(verum_ast::decl::FunctionBody::Expr(e)) => {
                    Some(BodyAst::Expr(e.clone()))
                }
                Maybe::None => None,
            };
            (
                decl.name.name.clone(),
                &decl.attributes,
                body_src,
                sig_src,
                body_ast,
            )
        }
        ItemKind::Theorem(decl)
        | ItemKind::Lemma(decl)
        | ItemKind::Corollary(decl) => {
            let body_src = span_slice(source_text, decl.proposition.span);
            let sig_src = span_slice_range(
                source_text,
                item_start,
                decl.proposition.span.start,
            );
            // Theorem proposition is a single Expr (carried via Heap).
            let body_ast = Some(BodyAst::Expr((*decl.proposition).clone()));
            (
                decl.name.name.clone(),
                &decl.attributes,
                body_src,
                sig_src,
                body_ast,
            )
        }
        _ => return,
    };
    // Walk both outer item attrs and inner decl attrs — parser
    // can place markers on either side.
    for attrs in [&item.attributes, decl_attrs] {
        for attr in attrs.iter() {
            if let Maybe::Some(extract) = ExtractAttr::from_attribute(attr) {
                let realize = match &extract.realize {
                    Maybe::Some(t) => Some(t.as_str().to_string()),
                    Maybe::None => None,
                };
                out.push(ExtractRequest {
                    name: item_name.clone(),
                    source: rel_path.to_path_buf(),
                    kind: ExtractKind::Program,
                    target: extract.target,
                    body_source: body_source.clone(),
                    signature_source: signature_source.clone(),
                    visibility_keyword,
                    body_ast: body_ast.clone(),
                    realize,
                });
            } else if let Maybe::Some(witness) =
                ExtractWitnessAttr::from_attribute(attr)
            {
                let realize = match &witness.realize {
                    Maybe::Some(t) => Some(t.as_str().to_string()),
                    Maybe::None => None,
                };
                out.push(ExtractRequest {
                    name: item_name.clone(),
                    source: rel_path.to_path_buf(),
                    kind: ExtractKind::Witness,
                    target: witness.target,
                    body_source: body_source.clone(),
                    signature_source: signature_source.clone(),
                    visibility_keyword,
                    body_ast: body_ast.clone(),
                    realize,
                });
            } else if let Maybe::Some(contract) =
                ExtractContractAttr::from_attribute(attr)
            {
                let realize = match &contract.realize {
                    Maybe::Some(t) => Some(t.as_str().to_string()),
                    Maybe::None => None,
                };
                out.push(ExtractRequest {
                    name: item_name.clone(),
                    source: rel_path.to_path_buf(),
                    kind: ExtractKind::Contract,
                    target: contract.target,
                    body_source: body_source.clone(),
                    signature_source: signature_source.clone(),
                    visibility_keyword,
                    body_ast: body_ast.clone(),
                    realize,
                });
            }
        }
    }
}

/// mangle a Verum identifier for OCaml's
/// value namespace. OCaml requires lowercase-leading for value
/// bindings (let-bindings, function names); leading-uppercase
/// names refer to type constructors. We lowercase the first
/// character when emitting a function-binding to keep the OCaml
/// output well-formed even when the Verum identifier starts
/// uppercase (rare but legal).
fn mangle_ocaml_ident(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_uppercase() => {
            let mut out = String::with_capacity(name.len() + 1);
            out.push('_');
            out.push(c.to_ascii_lowercase());
            out.extend(chars);
            out
        }
        _ => name.to_string(),
    }
}

/// render the visibility keyword that should
/// prepend a captured signature when re-emitting an extracted
/// declaration. The AST `Span` for `Function`/`Theorem`/`Lemma`/
/// `Corollary` covers from the keyword (`fn` / `theorem` / …) onward —
/// the visibility modifier is parsed separately and stored on the
/// decl. We splice it back at emit time. `None` for `Private`
/// (default — no keyword in source).
fn visibility_keyword_for(
    vis: &verum_ast::decl::Visibility,
) -> Option<&'static str> {
    use verum_ast::decl::Visibility;
    match vis {
        Visibility::Public => Some("public"),
        Visibility::PublicCrate => Some("public(crate)"),
        Visibility::PublicSuper => Some("public(super)"),
        Visibility::PublicIn(_) => Some("public"),
        Visibility::Internal => Some("internal"),
        Visibility::Protected => Some("protected"),
        Visibility::Private => None,
    }
}

/// extract a `&str` slice from `source` using
/// a `Span`'s `start` / `end` byte offsets. Returns `None` when
/// the span is out of bounds (defensive — should never happen for
/// well-formed AST). The returned `String` is `trim()`-ed to
/// remove leading/trailing whitespace from block boundaries.
fn span_slice(source: &str, span: verum_ast::span::Span) -> Option<String> {
    span_slice_range(source, span.start, span.end)
}

fn span_slice_range(source: &str, start: u32, end: u32) -> Option<String> {
    let s = start as usize;
    let e = end as usize;
    if s > e || e > source.len() {
        return None;
    }
    Some(source[s..e].trim().to_string())
}

fn extension_for(target: ExtractTarget) -> &'static str {
    match target {
        ExtractTarget::Verum => "vr",
        ExtractTarget::OCaml => "ml",
        ExtractTarget::Lean => "lean",
        ExtractTarget::Coq => "v",
    }
}

/// emit a target-language extraction for the
/// request. When `body_source` is captured (Verum target), splice
/// the AST body verbatim so the extracted file is re-checkable by
/// `verum check`. For other targets the body source is recorded
/// in a metadata comment until V12.2 wires per-target lowering
/// (Verum AST → OCaml / Lean / Coq surface).
fn emit_scaffold(req: &ExtractRequest) -> String {
    let header_comment = comment_for_target(req.target);
    let has_body = req.body_source.is_some();
    let stage_label = if has_body && req.target == ExtractTarget::Verum {
        "re-checkable extraction"
    } else if has_body {
        "body captured; per-target lowering"
    } else {
        "no body — signature-only scaffold"
    };

    let mut out = String::new();
    out.push_str(&format!(
        "{} Extracted by `verum extract` ({})\n",
        header_comment, stage_label
    ));
    out.push_str(&format!(
        "{} Source declaration: {} :: {}\n",
        header_comment,
        req.name.as_str(),
        req.source.display(),
    ));
    out.push_str(&format!(
        "{} Extraction kind:    {}({})\n\n",
        header_comment,
        req.kind.as_str(),
        req.target.as_str()
    ));

    // `realize="<fn_name>"` short-circuits the body-synthesis path:
    // emit a thin per-target wrapper that delegates to the named
    // native function. This lets a verified specification bind to
    // a hand-written / runtime-intrinsic primitive without losing
    // the proof-checked surface signature.
    if let Some(native_fn) = &req.realize {
        emit_realize_wrapper(&mut out, req, native_fn, header_comment);
        return out;
    }

    match req.target {
        ExtractTarget::Verum => {
            // V12.1: when body source is captured, splice the
            // signature + body verbatim, prepending the visibility
            // modifier (which AST spans don't cover). The result
            // re-parses through `verum check` because the snippets
            // came from a successfully-parsed source file.
            if let (Some(sig), Some(body)) = (&req.signature_source, &req.body_source) {
                out.push_str("@extracted\n");
                if let Some(vis) = req.visibility_keyword {
                    out.push_str(vis);
                    out.push(' ');
                }
                out.push_str(sig);
                let needs_space = !sig.ends_with(' ') && !sig.ends_with('\n');
                if needs_space {
                    out.push(' ');
                }
                out.push_str(body);
                out.push('\n');
            } else {
                out.push_str(&format!(
                    "@extracted\npublic fn {}() {{ /* signature-only — no extractable body */ }}\n",
                    req.name.as_str()
                ));
            }
        }
        ExtractTarget::OCaml => {
            // V12.2: try to lower the captured body AST through
            // the Verum-AST → OCaml lowerer. On success, splice
            // the real OCaml expression as the function body. On
            // failure (unsupported construct), fall back to V12.1
            // metadata comment + stub.
            let lowered = req.body_ast.as_ref().and_then(|b| match b {
                BodyAst::Block(blk) => ocaml_lower::lower_block(blk),
                BodyAst::Expr(e) => ocaml_lower::lower_expr(e),
            });
            match lowered {
                Some(ocaml_body) => {
                    out.push_str(&format!(
                        "(* @extracted (lowered) *)\nlet {} () = {}\n",
                        mangle_ocaml_ident(req.name.as_str()),
                        ocaml_body
                    ));
                }
                None => {
                    // Metadata-comment fallback for constructs
                    // outside the lowerer's current coverage
                    // (closures / async / mutation / etc.).
                    if let Some(body) = &req.body_source {
                        out.push_str(&format!(
                            "(* @extracted body (Verum source — lowering pending):\n{}\n*)\n",
                            body
                        ));
                    }
                    out.push_str(&format!(
                        "let {} () = (* body pending *) ()\n",
                        mangle_ocaml_ident(req.name.as_str())
                    ));
                }
            }
        }
        ExtractTarget::Lean => {
            // V12.2: try Verum AST → Lean 4 lowering. Fall back to
            // V12.1 metadata comment + stub on unsupported.
            let lowered = req.body_ast.as_ref().and_then(|b| match b {
                BodyAst::Block(blk) => lean_lower::lower_block(blk),
                BodyAst::Expr(e) => lean_lower::lower_expr(e),
            });
            match lowered {
                Some(lean_body) => {
                    out.push_str(&format!(
                        "/-- @extracted (lowered) -/\ndef {} : Unit := {}\n",
                        req.name.as_str(),
                        lean_body
                    ));
                }
                None => {
                    if let Some(body) = &req.body_source {
                        out.push_str(&format!(
                            "/-- @extracted body (Verum source — lowering pending):\n{}\n-/\n",
                            body
                        ));
                    }
                    out.push_str(&format!(
                        "def {} : Unit := () -- body pending\n",
                        req.name.as_str()
                    ));
                }
            }
        }
        ExtractTarget::Coq => {
            // V12.2: try Verum AST → Coq lowering. Fall back to
            // V12.1 metadata comment + stub on unsupported.
            let lowered = req.body_ast.as_ref().and_then(|b| match b {
                BodyAst::Block(blk) => coq_lower::lower_block(blk),
                BodyAst::Expr(e) => coq_lower::lower_expr(e),
            });
            match lowered {
                Some(coq_body) => {
                    out.push_str(&format!(
                        "(* @extracted (lowered) *)\nDefinition {} := {}.\n",
                        req.name.as_str(),
                        coq_body
                    ));
                }
                None => {
                    if let Some(body) = &req.body_source {
                        out.push_str(&format!(
                            "(* @extracted body (Verum source — lowering pending):\n{}\n*)\n",
                            body
                        ));
                    }
                    out.push_str(&format!(
                        "Definition {} : unit := tt. (* body pending *)\n",
                        req.name.as_str()
                    ));
                }
            }
        }
    }
    out
}

fn comment_for_target(target: ExtractTarget) -> &'static str {
    match target {
        ExtractTarget::Verum => "//",
        ExtractTarget::OCaml | ExtractTarget::Coq => "(*",
        ExtractTarget::Lean => "--",
    }
}

/// Emit a per-target wrapper that delegates to a hand-written
/// native function. Used by the `@extract(realize="...")` directive.
///
/// The wrapper preserves the verified surface signature; the body
/// is a single call into `native_fn`. This pattern ships verified
/// specifications for primitives the runtime owns (intrinsic
/// crypto, foreign syscall wrappers, etc.) without losing
/// proof-checked types at the boundary.
fn emit_realize_wrapper(
    out: &mut String,
    req: &ExtractRequest,
    native_fn: &str,
    header_comment: &str,
) {
    out.push_str(&format!(
        "{} Realize binding: delegates to native `{}`.\n",
        header_comment, native_fn
    ));
    match req.target {
        ExtractTarget::Verum => {
            // The Verum-target realize wrapper splices the original
            // signature so `verum check` re-validates the surface;
            // the body is a single call into the native function.
            out.push_str("@extracted\n");
            if let Some(vis) = req.visibility_keyword {
                out.push_str(vis);
                out.push(' ');
            }
            if let Some(sig) = &req.signature_source {
                out.push_str(sig);
                let needs_space = !sig.ends_with(' ') && !sig.ends_with('\n');
                if needs_space {
                    out.push(' ');
                }
                // Verum surface wrapper. We emit a body that
                // forwards to the native fn; the verifier rechecks
                // the surface contract against the call.
                out.push_str(&format!("{{ {}() }}\n", native_fn));
            } else {
                out.push_str(&format!(
                    "public fn {}() {{ {}() }}\n",
                    req.name.as_str(),
                    native_fn
                ));
            }
        }
        ExtractTarget::OCaml => {
            out.push_str(&format!(
                "(* @extracted (realize) *)\nlet {} () = {} ()\n",
                mangle_ocaml_ident(req.name.as_str()),
                native_fn
            ));
        }
        ExtractTarget::Lean => {
            out.push_str(&format!(
                "/-- @extracted (realize) -/\ndef {} : Unit := {} ()\n",
                req.name.as_str(),
                native_fn
            ));
        }
        ExtractTarget::Coq => {
            out.push_str(&format!(
                "(* @extracted (realize) *)\nDefinition {} := {} tt.\n",
                req.name.as_str(),
                native_fn
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_for_each_target() {
        assert_eq!(extension_for(ExtractTarget::Verum), "vr");
        assert_eq!(extension_for(ExtractTarget::OCaml), "ml");
        assert_eq!(extension_for(ExtractTarget::Lean), "lean");
        assert_eq!(extension_for(ExtractTarget::Coq), "v");
    }

    #[test]
    fn comment_for_target_uses_correct_syntax() {
        assert_eq!(comment_for_target(ExtractTarget::Verum), "//");
        assert_eq!(comment_for_target(ExtractTarget::OCaml), "(*");
        assert_eq!(comment_for_target(ExtractTarget::Lean), "--");
        assert_eq!(comment_for_target(ExtractTarget::Coq), "(*");
    }

    #[test]
    fn extract_kind_stringifies_correctly() {
        assert_eq!(ExtractKind::Program.as_str(), "@extract");
        assert_eq!(ExtractKind::Witness.as_str(), "@extract_witness");
        assert_eq!(ExtractKind::Contract.as_str(), "@extract_contract");
    }

    #[test]
    fn emit_scaffold_verum_target_uses_verum_syntax() {
        let req = ExtractRequest {
            name: Text::from("plus_comm"),
            source: PathBuf::from("src/lib.vr"),
            kind: ExtractKind::Program,
            target: ExtractTarget::Verum,
            body_source: None,
            signature_source: None,
            visibility_keyword: Some("public"),
            body_ast: None,
            realize: None,
        };
        let body = emit_scaffold(&req);
        assert!(body.contains("@extracted"));
        assert!(body.contains("public fn plus_comm"));
        assert!(body.contains("Extracted by `verum extract`"));
        assert!(body.contains("@extract(verum)"));
    }

    #[test]
    fn emit_scaffold_lean_target_uses_lean_syntax() {
        let req = ExtractRequest {
            name: Text::from("yoneda"),
            source: PathBuf::from("src/main.vr"),
            kind: ExtractKind::Witness,
            target: ExtractTarget::Lean,
            body_source: None,
            signature_source: None,
            visibility_keyword: Some("public"),
            body_ast: None,
            realize: None,
        };
        let body = emit_scaffold(&req);
        assert!(body.contains("def yoneda"));
        assert!(body.contains("--"));
        assert!(body.contains("@extract_witness(lean)"));
    }

    #[test]
    fn emit_scaffold_coq_target_uses_definition() {
        let req = ExtractRequest {
            name: Text::from("foo"),
            source: PathBuf::from("foo.vr"),
            kind: ExtractKind::Contract,
            target: ExtractTarget::Coq,
            body_source: None,
            signature_source: None,
            visibility_keyword: Some("public"),
            body_ast: None,
            realize: None,
        };
        let body = emit_scaffold(&req);
        assert!(body.contains("Definition foo"));
        assert!(body.contains("@extract_contract(coq)"));
    }

    #[test]
    fn emit_scaffold_ocaml_target_uses_let() {
        let req = ExtractRequest {
            name: Text::from("decode"),
            source: PathBuf::from("dec.vr"),
            kind: ExtractKind::Program,
            target: ExtractTarget::OCaml,
            body_source: None,
            signature_source: None,
            visibility_keyword: Some("public"),
            body_ast: None,
            realize: None,
        };
        let body = emit_scaffold(&req);
        assert!(body.contains("let decode"));
    }

    #[test]
    fn emit_scaffold_realize_emits_ocaml_delegate() {
        let req = ExtractRequest {
            name: Text::from("decode"),
            source: PathBuf::from("dec.vr"),
            kind: ExtractKind::Program,
            target: ExtractTarget::OCaml,
            body_source: None,
            signature_source: None,
            visibility_keyword: Some("public"),
            body_ast: None,
            realize: Some("native_decode".to_string()),
        };
        let body = emit_scaffold(&req);
        assert!(body.contains("Realize binding"));
        assert!(body.contains("let decode () = native_decode ()"));
        // The realize path short-circuits the body-pending fallback.
        assert!(!body.contains("body pending"));
    }

    #[test]
    fn emit_scaffold_realize_emits_lean_delegate() {
        let req = ExtractRequest {
            name: Text::from("decode"),
            source: PathBuf::from("dec.vr"),
            kind: ExtractKind::Program,
            target: ExtractTarget::Lean,
            body_source: None,
            signature_source: None,
            visibility_keyword: Some("public"),
            body_ast: None,
            realize: Some("native_decode".to_string()),
        };
        let body = emit_scaffold(&req);
        assert!(body.contains("def decode : Unit := native_decode ()"));
    }

    #[test]
    fn emit_scaffold_realize_emits_coq_delegate() {
        let req = ExtractRequest {
            name: Text::from("decode"),
            source: PathBuf::from("dec.vr"),
            kind: ExtractKind::Program,
            target: ExtractTarget::Coq,
            body_source: None,
            signature_source: None,
            visibility_keyword: Some("public"),
            body_ast: None,
            realize: Some("native_decode".to_string()),
        };
        let body = emit_scaffold(&req);
        assert!(body.contains("Definition decode := native_decode tt"));
    }

    #[test]
    fn emit_scaffold_realize_emits_verum_wrapper_with_signature() {
        let req = ExtractRequest {
            name: Text::from("decode"),
            source: PathBuf::from("dec.vr"),
            kind: ExtractKind::Program,
            target: ExtractTarget::Verum,
            body_source: None,
            signature_source: Some("fn decode() -> Bool".to_string()),
            visibility_keyword: Some("public"),
            body_ast: None,
            realize: Some("native_decode".to_string()),
        };
        let body = emit_scaffold(&req);
        assert!(body.contains("@extracted"));
        assert!(body.contains("public fn decode() -> Bool"));
        assert!(body.contains("native_decode()"));
    }
}
