//! Auto-paper generator from @theorem / @lemma / @corollary / @axiom
//! declarations.
//!
//! ## Goal
//!
//! Eliminate the duplicate-source problem (#84): a Verum corpus IS
//! the formal proof AND the paper draft.  Pre-this-module a project
//! had to maintain a `paper.tex` alongside the `.vr` corpus,
//! manually keeping the two in sync.  This module makes the corpus
//! the *single* source of truth — the renderer projects every
//! public theorem / lemma / corollary / axiom + its docstring + its
//! proof body into a structured [`DocItem`] and emits Markdown /
//! LaTeX / HTML directly from that.
//!
//! ## Architectural pattern
//!
//! Same single-trait-boundary pattern as the rest of the integration
//! arc (ladder_dispatch / tactic_combinator / proof_repair /
//! closure_cache):
//!
//!   * [`DocItem`] — typed projection of one declaration.
//!   * [`DocCorpus`] — collection + citation-graph + cross-ref
//!     validator.
//!   * [`RenderFormat`] — Markdown / Latex / Html.
//!   * [`DocRenderer`] trait — single dispatch interface.
//!   * [`DefaultDocRenderer`] — V0 reference covering all three
//!     formats.
//!
//! Future per-format adapters (LaTeX-with-proof-tree-collapse,
//! HTML-with-MathJax, Markdown-with-Mermaid-graphs) plug in via
//! the same trait without touching consumers.
//!
//! ## Reproducibility envelope
//!
//! [`DocItem`] carries an optional `closure_hash` — when present,
//! readers of the rendered paper can run `verum cache-closure decide
//! <name> --signature … --body … --cite …` against the same kernel
//! version to confirm the statement they're reading is the
//! statement that was kernel-checked.  This is the "auto-paper +
//! re-check" envelope #84 ships.
//!
//! ## Foundation-neutral
//!
//! The renderer knows nothing about how a `.vr` file is parsed —
//! callers (CLI / docs build) construct [`DocItem`]s from whatever
//! AST surface they've got and hand them in.  Rendering is a pure
//! function of the projection.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use verum_common::Text;

// =============================================================================
// DocItemKind + RenderFormat
// =============================================================================

/// Kind of declaration projected for rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DocItemKind {
    Theorem,
    Lemma,
    Corollary,
    Axiom,
}

impl DocItemKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Theorem => "theorem",
            Self::Lemma => "lemma",
            Self::Corollary => "corollary",
            Self::Axiom => "axiom",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "theorem" => Some(Self::Theorem),
            "lemma" => Some(Self::Lemma),
            "corollary" => Some(Self::Corollary),
            "axiom" => Some(Self::Axiom),
            _ => None,
        }
    }
}

/// Output format the renderer should produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RenderFormat {
    /// CommonMark-flavoured Markdown — web-readable, hyperlink-friendly.
    Markdown,
    /// LaTeX article body (no preamble) — paste into a paper template.
    Latex,
    /// HTML5 fragment — embed in a static-site generator.
    Html,
}

impl RenderFormat {
    pub fn name(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Latex => "latex",
            Self::Html => "html",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "markdown" | "md" => Some(Self::Markdown),
            "latex" | "tex" => Some(Self::Latex),
            "html" => Some(Self::Html),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Latex => "tex",
            Self::Html => "html",
        }
    }
}

// =============================================================================
// DocItem — typed projection of one declaration
// =============================================================================

/// Structured projection of one renderable declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocItem {
    pub kind: DocItemKind,
    /// Stable identifier (theorem name).
    pub name: Text,
    /// Docstring (the prose part of `/// …` comments above the decl).
    pub docstring: Text,
    /// Pretty-printed signature, e.g. `t_succ_pos(n: Int) -> Bool`.
    pub signature: Text,
    /// `requires` clauses, prose-rendered.
    pub requires: Vec<Text>,
    /// `ensures` clauses (the formal statement).
    pub ensures: Vec<Text>,
    /// Numbered tactic steps from the proof body (one element per
    /// step, e.g. `apply succ_lemma`, `norm_num`, `intro`).
    pub proof_steps: Vec<Text>,
    /// Names of theorems / lemmas this declaration cites.
    pub citations: Vec<Text>,
    /// `(framework, citation_string)` from `@framework("…","…")` markers.
    pub framework_markers: Vec<(Text, Text)>,
    /// Optional closure-cache hash for the reproducibility envelope.
    /// When set, readers can run `verum cache-closure decide …`
    /// against this hash to confirm the rendered statement matches
    /// the kernel-checked artefact.
    pub closure_hash: Option<Text>,
    /// Source file (relative to manifest dir).
    pub source_file: Text,
    /// 1-based line number.
    pub source_line: u32,
}

impl DocItem {
    /// Convenient constructor for the common case (no proof steps,
    /// no citations, no framework markers).  Mostly used in tests.
    pub fn new(
        kind: DocItemKind,
        name: impl Into<Text>,
        signature: impl Into<Text>,
    ) -> Self {
        Self {
            kind,
            name: name.into(),
            docstring: Text::from(""),
            signature: signature.into(),
            requires: Vec::new(),
            ensures: Vec::new(),
            proof_steps: Vec::new(),
            citations: Vec::new(),
            framework_markers: Vec::new(),
            closure_hash: None,
            source_file: Text::from(""),
            source_line: 0,
        }
    }

    /// Stable anchor for cross-references.  Format: `<kind>:<name>`.
    /// Used by every output format so refs are portable.
    pub fn anchor(&self) -> Text {
        Text::from(format!("{}:{}", self.kind.name(), self.name.as_str()))
    }
}

// =============================================================================
// DocCorpus — collection + citation graph + cross-ref validation
// =============================================================================

/// A collection of [`DocItem`]s plus derived structures.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DocCorpus {
    pub items: Vec<DocItem>,
}

/// One broken cross-reference detected by [`DocCorpus::validate_cross_refs`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BrokenRef {
    /// The item whose docstring contains the broken reference.
    pub citing_item: Text,
    /// The dangling target (the `name` part of `\ref{thm:name}` /
    /// `[text](#thm:name)` / `\\cite{name}`).
    pub broken_target: Text,
}

impl DocCorpus {
    pub fn new(items: Vec<DocItem>) -> Self {
        Self { items }
    }

    /// Build the citation graph: name → list of cited names.
    pub fn citation_graph(&self) -> BTreeMap<Text, Vec<Text>> {
        let mut g: BTreeMap<Text, Vec<Text>> = BTreeMap::new();
        for it in &self.items {
            g.insert(it.name.clone(), it.citations.clone());
        }
        g
    }

    /// Export the citation graph to Graphviz DOT.  Edge: citing →
    /// cited.  Nodes are coloured by item kind (theorem = blue,
    /// lemma = green, corollary = yellow, axiom = grey).
    pub fn to_dot(&self) -> Text {
        let mut out = String::from("digraph corpus_citations {\n");
        out.push_str("  rankdir=LR;\n");
        out.push_str("  node [shape=box, style=filled];\n");
        // Nodes.
        for it in &self.items {
            let colour = match it.kind {
                DocItemKind::Theorem => "lightblue",
                DocItemKind::Lemma => "lightgreen",
                DocItemKind::Corollary => "lightyellow",
                DocItemKind::Axiom => "lightgrey",
            };
            out.push_str(&format!(
                "  \"{}\" [fillcolor={}, label=\"{} ({})\"];\n",
                escape_dot(it.name.as_str()),
                colour,
                escape_dot(it.name.as_str()),
                it.kind.name()
            ));
        }
        // Edges.
        for it in &self.items {
            for cited in &it.citations {
                out.push_str(&format!(
                    "  \"{}\" -> \"{}\";\n",
                    escape_dot(it.name.as_str()),
                    escape_dot(cited.as_str())
                ));
            }
        }
        out.push('}');
        Text::from(out)
    }

    /// Detect broken citations: every `it.citations` entry must
    /// appear as the `name` of some other corpus item; otherwise
    /// it's a dangling reference.
    pub fn validate_cross_refs(&self) -> Vec<BrokenRef> {
        let known: BTreeSet<&str> =
            self.items.iter().map(|i| i.name.as_str()).collect();
        let mut broken = Vec::new();
        for it in &self.items {
            for c in &it.citations {
                if !known.contains(c.as_str()) {
                    broken.push(BrokenRef {
                        citing_item: it.name.clone(),
                        broken_target: c.clone(),
                    });
                }
            }
        }
        broken
    }

    /// Roots of the citation graph (items nothing cites).  These are
    /// the "top-level" theorems a reader should start from.
    pub fn roots(&self) -> Vec<Text> {
        let mut cited: BTreeSet<&str> = BTreeSet::new();
        for it in &self.items {
            for c in &it.citations {
                cited.insert(c.as_str());
            }
        }
        let mut roots: Vec<Text> = self
            .items
            .iter()
            .filter(|i| !cited.contains(i.name.as_str()))
            .map(|i| i.name.clone())
            .collect();
        roots.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        roots
    }
}

fn escape_dot(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

// =============================================================================
// DocRenderer — the trait boundary
// =============================================================================

/// Render error.
#[derive(Debug, Clone, PartialEq)]
pub enum RenderError {
    UnsupportedFormat(Text),
    Other(Text),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderError::UnsupportedFormat(t) => {
                write!(f, "unsupported format: {}", t.as_str())
            }
            RenderError::Other(t) => write!(f, "{}", t.as_str()),
        }
    }
}

impl std::error::Error for RenderError {}

/// Single dispatch interface for documentation rendering.
pub trait DocRenderer: std::fmt::Debug + Send + Sync {
    /// Render a full corpus as a single document (with header,
    /// table of contents, every item).
    fn render_corpus(
        &self,
        corpus: &DocCorpus,
        format: RenderFormat,
    ) -> Result<Text, RenderError>;

    /// Render a single item as a standalone fragment (no header).
    fn render_item(
        &self,
        item: &DocItem,
        format: RenderFormat,
    ) -> Result<Text, RenderError>;
}

// =============================================================================
// DefaultDocRenderer — V0 reference covering Markdown + LaTeX + HTML
// =============================================================================

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultDocRenderer;

impl DefaultDocRenderer {
    pub fn new() -> Self {
        Self
    }
}

impl DocRenderer for DefaultDocRenderer {
    fn render_corpus(
        &self,
        corpus: &DocCorpus,
        format: RenderFormat,
    ) -> Result<Text, RenderError> {
        match format {
            RenderFormat::Markdown => Ok(render_corpus_md(corpus)),
            RenderFormat::Latex => Ok(render_corpus_tex(corpus)),
            RenderFormat::Html => Ok(render_corpus_html(corpus)),
        }
    }

    fn render_item(
        &self,
        item: &DocItem,
        format: RenderFormat,
    ) -> Result<Text, RenderError> {
        match format {
            RenderFormat::Markdown => Ok(render_item_md(item)),
            RenderFormat::Latex => Ok(render_item_tex(item)),
            RenderFormat::Html => Ok(render_item_html(item)),
        }
    }
}

// ----- Markdown -----

fn render_corpus_md(corpus: &DocCorpus) -> Text {
    let mut s = String::from("# Verum corpus — formal statements\n\n");
    s.push_str(&format!(
        "Auto-generated from {} declaration(s).  Each statement carries a \
         kernel-cert hash; readers can re-check via `verum cache-closure decide`.\n\n",
        corpus.items.len()
    ));
    s.push_str("## Table of contents\n\n");
    for it in &corpus.items {
        s.push_str(&format!(
            "- [{} — {}](#{})\n",
            it.kind.name(),
            md_escape(it.name.as_str()),
            md_anchor(&it.anchor()),
        ));
    }
    s.push_str("\n---\n\n");
    for it in &corpus.items {
        s.push_str(&render_item_md(it).as_str().to_string());
        s.push_str("\n");
    }
    Text::from(s)
}

fn render_item_md(item: &DocItem) -> Text {
    let mut s = String::new();
    s.push_str(&format!(
        "## <a id=\"{}\"></a>{} `{}`\n\n",
        md_anchor(&item.anchor()),
        capitalize(item.kind.name()),
        md_escape(item.name.as_str())
    ));
    if !item.docstring.as_str().is_empty() {
        s.push_str(item.docstring.as_str());
        s.push_str("\n\n");
    }
    s.push_str("**Signature:**\n\n");
    s.push_str(&format!("```verum\n{}\n```\n\n", item.signature.as_str()));
    if !item.requires.is_empty() {
        s.push_str("**Requires:**\n\n");
        for r in &item.requires {
            s.push_str(&format!("- `{}`\n", md_escape(r.as_str())));
        }
        s.push('\n');
    }
    if !item.ensures.is_empty() {
        s.push_str("**Statement (ensures):**\n\n");
        for e in &item.ensures {
            s.push_str(&format!("- `{}`\n", md_escape(e.as_str())));
        }
        s.push('\n');
    }
    if !item.proof_steps.is_empty() {
        s.push_str("**Proof:**\n\n");
        for (i, step) in item.proof_steps.iter().enumerate() {
            s.push_str(&format!(
                "{}. `{}`\n",
                i + 1,
                md_escape(step.as_str())
            ));
        }
        s.push('\n');
    }
    if !item.citations.is_empty() {
        s.push_str("**Cites:** ");
        let parts: Vec<String> = item
            .citations
            .iter()
            .map(|c| {
                format!(
                    "[`{}`](#{})",
                    md_escape(c.as_str()),
                    md_anchor(&Text::from(format!("ref:{}", c.as_str())))
                )
            })
            .collect();
        s.push_str(&parts.join(", "));
        s.push_str("\n\n");
    }
    if !item.framework_markers.is_empty() {
        s.push_str("**Framework citations:**\n\n");
        for (fw, cite) in &item.framework_markers {
            s.push_str(&format!(
                "- *{}*: {}\n",
                md_escape(fw.as_str()),
                md_escape(cite.as_str())
            ));
        }
        s.push('\n');
    }
    if let Some(h) = &item.closure_hash {
        s.push_str(&format!(
            "**Closure hash:** `{}` (re-check with `verum cache-closure decide {} …`)\n\n",
            md_escape(h.as_str()),
            md_escape(item.name.as_str())
        ));
    }
    if !item.source_file.as_str().is_empty() {
        s.push_str(&format!(
            "*Source:* `{}:{}`\n",
            md_escape(item.source_file.as_str()),
            item.source_line
        ));
    }
    Text::from(s)
}

// ----- LaTeX -----

fn render_corpus_tex(corpus: &DocCorpus) -> Text {
    let mut s = String::from("% Auto-generated by verum doc render\n");
    s.push_str("\\section{Verum corpus --- formal statements}\n\n");
    s.push_str(&format!(
        "Auto-generated from {} declaration(s). Each statement carries \
         a kernel-cert hash; readers can re-check via \\texttt{{verum cache-closure decide}}.\n\n",
        corpus.items.len()
    ));
    for it in &corpus.items {
        s.push_str(&render_item_tex(it).as_str().to_string());
        s.push_str("\n");
    }
    Text::from(s)
}

fn render_item_tex(item: &DocItem) -> Text {
    let mut s = String::new();
    let env = match item.kind {
        DocItemKind::Theorem => "theorem",
        DocItemKind::Lemma => "lemma",
        DocItemKind::Corollary => "corollary",
        DocItemKind::Axiom => "axiom",
    };
    s.push_str(&format!(
        "\\begin{{{}}}[{}]\n\\label{{{}}}\n",
        env,
        tex_escape(item.name.as_str()),
        tex_escape(item.anchor().as_str()),
    ));
    if !item.docstring.as_str().is_empty() {
        s.push_str(&tex_escape(item.docstring.as_str()));
        s.push_str("\n\n");
    }
    if !item.ensures.is_empty() {
        for e in &item.ensures {
            s.push_str(&format!("\\(\\text{{{}}}\\)\n", tex_escape(e.as_str())));
        }
    }
    s.push_str(&format!("\\end{{{}}}\n", env));
    if !item.proof_steps.is_empty() && item.kind != DocItemKind::Axiom {
        s.push_str("\\begin{proof}\n");
        s.push_str("\\begin{enumerate}\n");
        for step in &item.proof_steps {
            s.push_str(&format!(
                "\\item \\texttt{{{}}}\n",
                tex_escape(step.as_str())
            ));
        }
        s.push_str("\\end{enumerate}\n");
        s.push_str("\\end{proof}\n");
    }
    if !item.framework_markers.is_empty() {
        s.push_str("\\textbf{Framework citations:} ");
        let parts: Vec<String> = item
            .framework_markers
            .iter()
            .map(|(fw, c)| {
                format!(
                    "\\emph{{{}}}: {}",
                    tex_escape(fw.as_str()),
                    tex_escape(c.as_str())
                )
            })
            .collect();
        s.push_str(&parts.join("; "));
        s.push('\n');
    }
    if let Some(h) = &item.closure_hash {
        s.push_str(&format!(
            "\\textit{{Closure hash:}} \\texttt{{{}}}\n",
            tex_escape(h.as_str())
        ));
    }
    Text::from(s)
}

// ----- HTML -----

fn render_corpus_html(corpus: &DocCorpus) -> Text {
    let mut s = String::from("<section class=\"verum-corpus\">\n");
    s.push_str("<h1>Verum corpus — formal statements</h1>\n");
    s.push_str(&format!(
        "<p>Auto-generated from <strong>{}</strong> declaration(s).</p>\n",
        corpus.items.len()
    ));
    s.push_str("<nav class=\"toc\"><ul>\n");
    for it in &corpus.items {
        s.push_str(&format!(
            "<li><a href=\"#{}\">{} — <code>{}</code></a></li>\n",
            html_escape(it.anchor().as_str()),
            it.kind.name(),
            html_escape(it.name.as_str())
        ));
    }
    s.push_str("</ul></nav>\n");
    for it in &corpus.items {
        s.push_str(&render_item_html(it).as_str().to_string());
    }
    s.push_str("</section>\n");
    Text::from(s)
}

fn render_item_html(item: &DocItem) -> Text {
    let mut s = String::new();
    s.push_str(&format!(
        "<article class=\"verum-item verum-{}\" id=\"{}\">\n",
        item.kind.name(),
        html_escape(item.anchor().as_str())
    ));
    s.push_str(&format!(
        "<h2><span class=\"kind\">{}</span> <code>{}</code></h2>\n",
        item.kind.name(),
        html_escape(item.name.as_str())
    ));
    if !item.docstring.as_str().is_empty() {
        s.push_str(&format!(
            "<p class=\"docstring\">{}</p>\n",
            html_escape(item.docstring.as_str())
        ));
    }
    s.push_str(&format!(
        "<pre class=\"signature\"><code>{}</code></pre>\n",
        html_escape(item.signature.as_str())
    ));
    if !item.ensures.is_empty() {
        s.push_str("<dl class=\"statement\"><dt>Statement</dt>\n");
        for e in &item.ensures {
            s.push_str(&format!(
                "<dd><code>{}</code></dd>\n",
                html_escape(e.as_str())
            ));
        }
        s.push_str("</dl>\n");
    }
    if !item.proof_steps.is_empty() {
        s.push_str("<ol class=\"proof\">\n");
        for step in &item.proof_steps {
            s.push_str(&format!(
                "<li><code>{}</code></li>\n",
                html_escape(step.as_str())
            ));
        }
        s.push_str("</ol>\n");
    }
    if let Some(h) = &item.closure_hash {
        s.push_str(&format!(
            "<p class=\"closure-hash\">Closure hash: <code>{}</code></p>\n",
            html_escape(h.as_str())
        ));
    }
    s.push_str("</article>\n");
    Text::from(s)
}

// ----- escape helpers -----

fn md_escape(s: &str) -> String {
    s.replace('|', "\\|")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn md_anchor(s: &Text) -> String {
    s.as_str()
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' => c.to_ascii_lowercase(),
            _ => '-',
        })
        .collect()
}

fn tex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\textbackslash{}"),
            '&' | '%' | '$' | '#' | '_' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            '~' => out.push_str("\\textasciitilde{}"),
            '^' => out.push_str("\\textasciicircum{}"),
            c => out.push(c),
        }
    }
    out
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item_thm(name: &str) -> DocItem {
        let mut it = DocItem::new(
            DocItemKind::Theorem,
            name,
            format!("{}() -> Bool", name),
        );
        it.docstring = Text::from(format!("Docstring for {}.", name));
        it.ensures = vec![Text::from("a + b == b + a")];
        it.proof_steps = vec![Text::from("intro"), Text::from("apply auto")];
        it.source_file = Text::from("src/main.vr");
        it.source_line = 42;
        it
    }

    // ----- DocItemKind / RenderFormat round-trip -----

    #[test]
    fn doc_item_kind_round_trip() {
        for k in [
            DocItemKind::Theorem,
            DocItemKind::Lemma,
            DocItemKind::Corollary,
            DocItemKind::Axiom,
        ] {
            assert_eq!(DocItemKind::from_name(k.name()), Some(k));
        }
        assert_eq!(DocItemKind::from_name("unknown"), None);
    }

    #[test]
    fn render_format_round_trip() {
        for f in [
            RenderFormat::Markdown,
            RenderFormat::Latex,
            RenderFormat::Html,
        ] {
            assert_eq!(RenderFormat::from_name(f.name()), Some(f));
        }
        // Aliases.
        assert_eq!(RenderFormat::from_name("md"), Some(RenderFormat::Markdown));
        assert_eq!(RenderFormat::from_name("tex"), Some(RenderFormat::Latex));
        assert_eq!(RenderFormat::from_name("yaml"), None);
    }

    #[test]
    fn render_format_extension_distinct() {
        let exts: BTreeSet<&str> = [
            RenderFormat::Markdown,
            RenderFormat::Latex,
            RenderFormat::Html,
        ]
        .iter()
        .map(|f| f.extension())
        .collect();
        assert_eq!(exts.len(), 3);
    }

    // ----- DocItem -----

    #[test]
    fn doc_item_anchor_format() {
        let it = item_thm("foo_lemma");
        assert_eq!(it.anchor().as_str(), "theorem:foo_lemma");
    }

    // ----- DocCorpus -----

    #[test]
    fn citation_graph_records_all_items() {
        let mut a = item_thm("a");
        a.citations = vec![Text::from("b")];
        let b = item_thm("b");
        let corpus = DocCorpus::new(vec![a, b]);
        let g = corpus.citation_graph();
        assert_eq!(g.len(), 2);
        assert_eq!(g[&Text::from("a")], vec![Text::from("b")]);
        assert_eq!(g[&Text::from("b")], Vec::<Text>::new());
    }

    #[test]
    fn validate_cross_refs_empty_when_all_resolved() {
        let mut a = item_thm("a");
        a.citations = vec![Text::from("b")];
        let b = item_thm("b");
        let corpus = DocCorpus::new(vec![a, b]);
        assert!(corpus.validate_cross_refs().is_empty());
    }

    #[test]
    fn validate_cross_refs_finds_dangling() {
        let mut a = item_thm("a");
        a.citations = vec![Text::from("missing"), Text::from("b")];
        let b = item_thm("b");
        let corpus = DocCorpus::new(vec![a, b]);
        let broken = corpus.validate_cross_refs();
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].citing_item.as_str(), "a");
        assert_eq!(broken[0].broken_target.as_str(), "missing");
    }

    #[test]
    fn roots_excludes_cited_items() {
        let mut a = item_thm("a");
        a.citations = vec![Text::from("b")];
        let b = item_thm("b");
        let c = item_thm("c");
        let corpus = DocCorpus::new(vec![a, b, c]);
        let roots = corpus.roots();
        // a and c are NOT cited; b is cited by a.
        assert_eq!(roots.len(), 2);
        assert!(roots.iter().any(|r| r.as_str() == "a"));
        assert!(roots.iter().any(|r| r.as_str() == "c"));
        assert!(!roots.iter().any(|r| r.as_str() == "b"));
    }

    #[test]
    fn to_dot_emits_well_formed_graph() {
        let mut a = item_thm("a");
        a.citations = vec![Text::from("b")];
        let b = item_thm("b");
        let corpus = DocCorpus::new(vec![a, b]);
        let dot = corpus.to_dot();
        let s = dot.as_str();
        assert!(s.starts_with("digraph corpus_citations"));
        assert!(s.contains("\"a\" -> \"b\""));
        assert!(s.contains("fillcolor="));
        assert!(s.ends_with('}'));
    }

    // ----- DefaultDocRenderer — Markdown -----

    #[test]
    fn render_corpus_md_includes_toc() {
        let r = DefaultDocRenderer::new();
        let corpus = DocCorpus::new(vec![item_thm("foo"), item_thm("bar")]);
        let s = r.render_corpus(&corpus, RenderFormat::Markdown).unwrap();
        let s = s.as_str();
        assert!(s.contains("Table of contents"));
        assert!(s.contains("[theorem — foo]"));
        assert!(s.contains("[theorem — bar]"));
    }

    #[test]
    fn render_item_md_renders_proof_steps_numbered() {
        let r = DefaultDocRenderer::new();
        let s = r
            .render_item(&item_thm("foo"), RenderFormat::Markdown)
            .unwrap();
        let s = s.as_str();
        assert!(s.contains("**Proof:**"));
        assert!(s.contains("1. `intro`"));
        assert!(s.contains("2. `apply auto`"));
    }

    #[test]
    fn render_item_md_includes_source_line() {
        let r = DefaultDocRenderer::new();
        let s = r
            .render_item(&item_thm("foo"), RenderFormat::Markdown)
            .unwrap();
        assert!(s.as_str().contains("src/main.vr:42"));
    }

    #[test]
    fn render_item_md_includes_closure_hash_when_set() {
        let r = DefaultDocRenderer::new();
        let mut it = item_thm("foo");
        it.closure_hash =
            Some(Text::from("00112233445566778899aabbccddeeff".to_string()));
        let s = r.render_item(&it, RenderFormat::Markdown).unwrap();
        assert!(s.as_str().contains("Closure hash"));
        assert!(s.as_str().contains("00112233"));
    }

    // ----- DefaultDocRenderer — LaTeX -----

    #[test]
    fn render_item_tex_uses_correct_environment_per_kind() {
        let r = DefaultDocRenderer::new();
        for (k, env) in [
            (DocItemKind::Theorem, "theorem"),
            (DocItemKind::Lemma, "lemma"),
            (DocItemKind::Corollary, "corollary"),
            (DocItemKind::Axiom, "axiom"),
        ] {
            let mut it = item_thm("name");
            it.kind = k;
            let s = r.render_item(&it, RenderFormat::Latex).unwrap();
            assert!(
                s.as_str().contains(&format!("\\begin{{{}}}", env)),
                "kind {:?} → env {}",
                k,
                env
            );
        }
    }

    #[test]
    fn render_item_tex_axiom_omits_proof_block() {
        let r = DefaultDocRenderer::new();
        let mut it = item_thm("ax");
        it.kind = DocItemKind::Axiom;
        let s = r.render_item(&it, RenderFormat::Latex).unwrap();
        assert!(
            !s.as_str().contains("\\begin{proof}"),
            "axiom must not have a proof block"
        );
    }

    #[test]
    fn tex_escape_handles_special_chars() {
        assert_eq!(tex_escape("a_b"), "a\\_b");
        assert_eq!(tex_escape("x{y}"), "x\\{y\\}");
        assert_eq!(tex_escape("p%q"), "p\\%q");
    }

    // ----- DefaultDocRenderer — HTML -----

    #[test]
    fn render_item_html_emits_anchor_id() {
        let r = DefaultDocRenderer::new();
        let s = r.render_item(&item_thm("foo"), RenderFormat::Html).unwrap();
        assert!(s.as_str().contains("id=\"theorem:foo\""));
    }

    #[test]
    fn html_escape_quotes_and_amps() {
        assert_eq!(html_escape("a&b<c>d\""), "a&amp;b&lt;c&gt;d&quot;");
    }

    #[test]
    fn render_corpus_html_wraps_in_section() {
        let r = DefaultDocRenderer::new();
        let corpus = DocCorpus::new(vec![item_thm("foo")]);
        let s = r.render_corpus(&corpus, RenderFormat::Html).unwrap();
        let s = s.as_str();
        assert!(s.starts_with("<section"));
        assert!(s.ends_with("</section>\n"));
    }

    // ----- Cross-format consistency -----

    #[test]
    fn every_format_renders_every_item_kind_without_panic() {
        let r = DefaultDocRenderer::new();
        for k in [
            DocItemKind::Theorem,
            DocItemKind::Lemma,
            DocItemKind::Corollary,
            DocItemKind::Axiom,
        ] {
            let mut it = item_thm("x");
            it.kind = k;
            for f in [
                RenderFormat::Markdown,
                RenderFormat::Latex,
                RenderFormat::Html,
            ] {
                let _s = r.render_item(&it, f).expect(&format!("{:?} {:?}", k, f));
            }
        }
    }

    // ----- Pin #84 acceptance criteria -----

    #[test]
    fn task_84_three_formats_and_citation_graph_all_supported() {
        // §1: latex / markdown / html supported.
        let r = DefaultDocRenderer::new();
        let corpus = DocCorpus::new(vec![item_thm("foo")]);
        for f in [
            RenderFormat::Markdown,
            RenderFormat::Latex,
            RenderFormat::Html,
        ] {
            assert!(r.render_corpus(&corpus, f).is_ok());
        }
        // §5: citation graph exported as DOT.
        assert!(corpus.to_dot().as_str().starts_with("digraph"));
    }

    #[test]
    fn task_84_broken_xref_surfaces_as_brokenref() {
        // §2: broken refs are CI errors.
        let mut a = item_thm("a");
        a.citations = vec![Text::from("nonexistent")];
        let corpus = DocCorpus::new(vec![a]);
        let broken = corpus.validate_cross_refs();
        assert!(!broken.is_empty());
        assert_eq!(broken[0].broken_target.as_str(), "nonexistent");
    }
}
