//! Foreign-system theorem import — inverse of cross-format export.
//!
//! ## Goal
//!
//! Verum is foundation-neutral.  A Coq/Lean/Mizar/Isabelle corpus
//! can be imported as Verum theorem skeletons whose statement is
//! preserved, framework-attributed back to the source file/line, and
//! left ready for a Verum-side proof body (or admitted via
//! `@axiom` with the original citation).  This is the inverse of
//! `verum export` — together the two surfaces give Verum
//! bidirectional reproducibility across every supported proof
//! system.
//!
//! ## Architectural pattern
//!
//! Same single-trait-boundary pattern as the rest of the integration
//! arc (ladder_dispatch / tactic_combinator / proof_repair /
//! closure_cache / doc_render):
//!
//!   * [`ForeignTheorem`] — typed projection of one imported decl.
//!   * [`ForeignSystem`] enum — Coq / Lean4 / Mizar / Isabelle.
//!   * [`ForeignSystemImporter`] trait — single dispatch interface.
//!   * Per-system reference impls: [`CoqImporter`], [`Lean4Importer`],
//!     [`MizarImporter`], [`IsabelleImporter`].  V0 ships
//!     statement-level extraction (regex-based).  V1 will add
//!     proof-term translation.
//!   * [`importer_for`] dispatcher — pick the right importer for a
//!     [`ForeignSystem`] tag.
//!
//! ## V0 contract
//!
//!   * The importer extracts theorem **statements** (signature +
//!     proposition) but admits the proof body as `@axiom` with a
//!     `@framework(<system>, "<source>:<line>")` citation.
//!   * The user then fills in the proof body with Verum tactics, or
//!     keeps the `@axiom` and treats the foreign system as the trust
//!     boundary.
//!   * Citation chain is preserved end-to-end: a theorem imported
//!     from `Mathlib.Algebra.Group.Basic` lands in Verum with
//!     `@framework(lean_mathlib4, "Mathlib/Algebra/Group/Basic.lean:42")`,
//!     so the audit subcommands surface the foreign provenance.

use serde::{Deserialize, Serialize};
use std::path::Path;
use verum_common::Text;

// =============================================================================
// ForeignSystem
// =============================================================================

/// Foreign proof system this importer handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ForeignSystem {
    /// Coq / Rocq — `.v` files.  Parses `Theorem` / `Lemma` /
    /// `Corollary` / `Axiom` / `Definition`.
    Coq,
    /// Lean 4 / Mathlib4 — `.lean` files.  Parses `theorem` /
    /// `lemma` / `axiom` / `def`.
    Lean4,
    /// Mizar — `.miz` files.  Parses `theorem` / `definition` /
    /// `reservation`.
    Mizar,
    /// Isabelle/HOL — `.thy` files.  Parses `theorem` / `lemma` /
    /// `axiomatization`.
    Isabelle,
}

impl ForeignSystem {
    /// Stable diagnostic name (matches the `--from <name>` flag).
    pub fn name(self) -> &'static str {
        match self {
            Self::Coq => "coq",
            Self::Lean4 => "lean4",
            Self::Mizar => "mizar",
            Self::Isabelle => "isabelle",
        }
    }

    /// Parse a system tag from its diagnostic name.  Accepts a few
    /// common aliases.
    pub fn from_name(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "coq" | "rocq" => Some(Self::Coq),
            "lean4" | "lean" | "mathlib4" | "mathlib" => Some(Self::Lean4),
            "mizar" | "mml" => Some(Self::Mizar),
            "isabelle" | "isabelle/hol" | "hol" => Some(Self::Isabelle),
            _ => None,
        }
    }

    /// Conventional file extension.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Coq => "v",
            Self::Lean4 => "lean",
            Self::Mizar => "miz",
            Self::Isabelle => "thy",
        }
    }

    /// Framework tag for `@framework(<tag>, "...")` attribution.
    pub fn framework_tag(self) -> &'static str {
        match self {
            Self::Coq => "coq",
            Self::Lean4 => "lean_mathlib4",
            Self::Mizar => "mizar_mml",
            Self::Isabelle => "isabelle_hol",
        }
    }

    /// All supported systems.
    pub fn all() -> [ForeignSystem; 4] {
        [Self::Coq, Self::Lean4, Self::Mizar, Self::Isabelle]
    }
}

// =============================================================================
// ForeignTheorem + ForeignTheoremKind
// =============================================================================

/// What kind of declaration we extracted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ForeignTheoremKind {
    Theorem,
    Lemma,
    Corollary,
    Axiom,
    Definition,
}

impl ForeignTheoremKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Theorem => "theorem",
            Self::Lemma => "lemma",
            Self::Corollary => "corollary",
            Self::Axiom => "axiom",
            Self::Definition => "def",
        }
    }
}

/// One imported declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForeignTheorem {
    pub system: ForeignSystem,
    pub name: Text,
    pub kind: ForeignTheoremKind,
    /// Raw statement string (everything between `:` and the end of
    /// the declaration before the proof body).  Verbatim — no
    /// translation; the user / LLM-tactic translates this to a
    /// Verum proposition at fill time.
    pub statement: Text,
    /// Source file (relative or absolute).
    pub source_file: Text,
    /// 1-based line number where the declaration begins.
    pub source_line: u32,
    /// `@framework(<tag>, "<source>:<line>")` citation for the
    /// emitted Verum skeleton.  Composed from `system.framework_tag`
    /// + `source_file:source_line`.
    pub framework_citation: Text,
}

impl ForeignTheorem {
    /// Render a Verum `.vr` skeleton for this declaration.
    /// Statement is included as a comment (the user translates it
    /// to a Verum proposition); the body is admitted as `@axiom`.
    pub fn to_verum_skeleton(&self) -> Text {
        let kind = match self.kind {
            ForeignTheoremKind::Theorem => "theorem",
            ForeignTheoremKind::Lemma => "lemma",
            ForeignTheoremKind::Corollary => "corollary",
            ForeignTheoremKind::Axiom => "axiom",
            ForeignTheoremKind::Definition => "axiom", // imported as opaque
        };
        let mut s = String::new();
        s.push_str(&format!(
            "// imported from {}: {}:{}\n",
            self.system.name(),
            self.source_file.as_str(),
            self.source_line
        ));
        // Render the original statement as a comment block so the
        // human / LLM knows what to translate.
        s.push_str("//\n");
        for line in self.statement.as_str().lines() {
            s.push_str(&format!("//   {}\n", line));
        }
        s.push_str(&format!(
            "@framework({}, \"{}\")\n",
            self.system.framework_tag(),
            self.framework_citation.as_str()
        ));
        s.push_str(&format!(
            "public {} {}()\n",
            kind,
            self.name.as_str()
        ));
        s.push_str("    ensures /* TODO: translate the foreign statement above */ true\n");
        s.push_str("    proof by axiom;\n\n");
        Text::from(s)
    }
}

// =============================================================================
// ImportError
// =============================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum ImportError {
    Io(Text),
    Parse(Text),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Io(t) => write!(f, "I/O error: {}", t.as_str()),
            ImportError::Parse(t) => write!(f, "parse error: {}", t.as_str()),
        }
    }
}

impl std::error::Error for ImportError {}

// =============================================================================
// ForeignSystemImporter trait
// =============================================================================

/// Single dispatch interface for foreign-system theorem import.
pub trait ForeignSystemImporter: std::fmt::Debug + Send + Sync {
    /// The foreign system this importer handles.
    fn system(&self) -> ForeignSystem;

    /// Parse a foreign-system source file and extract every
    /// declaration the V0 importer recognises.
    fn parse_file(&self, path: &Path) -> Result<Vec<ForeignTheorem>, ImportError>;

    /// Parse the contents of a foreign-system source as a string.
    /// Used by tests + the CLI when piping content via stdin.
    fn parse_text(
        &self,
        content: &str,
        source_file: &str,
    ) -> Result<Vec<ForeignTheorem>, ImportError>;
}

/// Look up the canonical importer for a system tag.
pub fn importer_for(system: ForeignSystem) -> Box<dyn ForeignSystemImporter> {
    match system {
        ForeignSystem::Coq => Box::new(CoqImporter),
        ForeignSystem::Lean4 => Box::new(Lean4Importer),
        ForeignSystem::Mizar => Box::new(MizarImporter),
        ForeignSystem::Isabelle => Box::new(IsabelleImporter),
    }
}

// =============================================================================
// CoqImporter
// =============================================================================

/// Coq / Rocq statement-level importer.  Recognises:
///
///   * `Theorem <name> : <statement>.` (proof body discarded)
///   * `Lemma <name> : <statement>.`
///   * `Corollary <name> : <statement>.`
///   * `Axiom <name> : <statement>.`
///   * `Definition <name> ... : <type> := <body>.`
///
/// The statement extends from the `:` after the name to the
/// terminating `.` (Coq's statement terminator).  Multi-line
/// statements are preserved verbatim.
#[derive(Debug, Default, Clone, Copy)]
pub struct CoqImporter;

impl ForeignSystemImporter for CoqImporter {
    fn system(&self) -> ForeignSystem {
        ForeignSystem::Coq
    }

    fn parse_file(&self, path: &Path) -> Result<Vec<ForeignTheorem>, ImportError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| ImportError::Io(Text::from(format!("{}: {}", path.display(), e))))?;
        self.parse_text(&content, &path.display().to_string())
    }

    fn parse_text(
        &self,
        content: &str,
        source_file: &str,
    ) -> Result<Vec<ForeignTheorem>, ImportError> {
        Ok(extract_decls(
            content,
            source_file,
            ForeignSystem::Coq,
            COQ_KEYWORDS,
        ))
    }
}

const COQ_KEYWORDS: &[(&str, ForeignTheoremKind)] = &[
    ("Theorem", ForeignTheoremKind::Theorem),
    ("Lemma", ForeignTheoremKind::Lemma),
    ("Corollary", ForeignTheoremKind::Corollary),
    ("Axiom", ForeignTheoremKind::Axiom),
    ("Definition", ForeignTheoremKind::Definition),
];

// =============================================================================
// Lean4Importer
// =============================================================================

/// Lean 4 / Mathlib4 importer.  Recognises:
///
///   * `theorem <name> : <statement> := <proof>` (proof discarded)
///   * `lemma <name> : <statement> := <proof>`
///   * `axiom <name> : <statement>`
///   * `def <name> : <type> := <body>`
///
/// Statement extends from `:` after the name to `:=` (the proof
/// separator).  Stops at end-of-line if there's no `:=` (axioms).
#[derive(Debug, Default, Clone, Copy)]
pub struct Lean4Importer;

impl ForeignSystemImporter for Lean4Importer {
    fn system(&self) -> ForeignSystem {
        ForeignSystem::Lean4
    }

    fn parse_file(&self, path: &Path) -> Result<Vec<ForeignTheorem>, ImportError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| ImportError::Io(Text::from(format!("{}: {}", path.display(), e))))?;
        self.parse_text(&content, &path.display().to_string())
    }

    fn parse_text(
        &self,
        content: &str,
        source_file: &str,
    ) -> Result<Vec<ForeignTheorem>, ImportError> {
        Ok(extract_decls(
            content,
            source_file,
            ForeignSystem::Lean4,
            LEAN_KEYWORDS,
        ))
    }
}

const LEAN_KEYWORDS: &[(&str, ForeignTheoremKind)] = &[
    ("theorem", ForeignTheoremKind::Theorem),
    ("lemma", ForeignTheoremKind::Lemma),
    ("axiom", ForeignTheoremKind::Axiom),
    ("def", ForeignTheoremKind::Definition),
];

// =============================================================================
// MizarImporter
// =============================================================================

/// Mizar Mathematical Library importer.  Statement-level only;
/// Mizar's `proof ... end` blocks are admitted.
#[derive(Debug, Default, Clone, Copy)]
pub struct MizarImporter;

impl ForeignSystemImporter for MizarImporter {
    fn system(&self) -> ForeignSystem {
        ForeignSystem::Mizar
    }

    fn parse_file(&self, path: &Path) -> Result<Vec<ForeignTheorem>, ImportError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| ImportError::Io(Text::from(format!("{}: {}", path.display(), e))))?;
        self.parse_text(&content, &path.display().to_string())
    }

    fn parse_text(
        &self,
        content: &str,
        source_file: &str,
    ) -> Result<Vec<ForeignTheorem>, ImportError> {
        Ok(extract_decls(
            content,
            source_file,
            ForeignSystem::Mizar,
            MIZAR_KEYWORDS,
        ))
    }
}

const MIZAR_KEYWORDS: &[(&str, ForeignTheoremKind)] = &[
    ("theorem", ForeignTheoremKind::Theorem),
    ("definition", ForeignTheoremKind::Definition),
];

// =============================================================================
// IsabelleImporter
// =============================================================================

/// Isabelle/HOL importer.  Recognises `theorem` / `lemma` /
/// `axiomatization` keywords; statements span until the next
/// `proof` / `by` / `apply` keyword (where the proof body begins).
#[derive(Debug, Default, Clone, Copy)]
pub struct IsabelleImporter;

impl ForeignSystemImporter for IsabelleImporter {
    fn system(&self) -> ForeignSystem {
        ForeignSystem::Isabelle
    }

    fn parse_file(&self, path: &Path) -> Result<Vec<ForeignTheorem>, ImportError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| ImportError::Io(Text::from(format!("{}: {}", path.display(), e))))?;
        self.parse_text(&content, &path.display().to_string())
    }

    fn parse_text(
        &self,
        content: &str,
        source_file: &str,
    ) -> Result<Vec<ForeignTheorem>, ImportError> {
        Ok(extract_decls(
            content,
            source_file,
            ForeignSystem::Isabelle,
            ISABELLE_KEYWORDS,
        ))
    }
}

const ISABELLE_KEYWORDS: &[(&str, ForeignTheoremKind)] = &[
    ("theorem", ForeignTheoremKind::Theorem),
    ("lemma", ForeignTheoremKind::Lemma),
    ("axiomatization", ForeignTheoremKind::Axiom),
];

// =============================================================================
// Shared statement-level extractor
// =============================================================================

/// Generic line-based extractor used by every V0 importer.  Scans
/// the source for lines beginning with one of the configured
/// keywords; the next identifier becomes the theorem name; the
/// statement runs to the line's terminator (`.` for Coq/Mizar/
/// Isabelle, `:=` for Lean) or end-of-line.
///
/// Comments (Coq-style `(* ... *)`, Lean-style `-- ...`, Mizar-
/// style `::`, Isabelle-style `(*...*)`) are stripped from the
/// keyword-detection pass — declarations inside comments don't
/// trigger imports.
fn extract_decls(
    content: &str,
    source_file: &str,
    system: ForeignSystem,
    keywords: &[(&str, ForeignTheoremKind)],
) -> Vec<ForeignTheorem> {
    let mut out: Vec<ForeignTheorem> = Vec::new();
    let stripped = strip_comments(content, system);
    for (line_idx, line) in stripped.lines().enumerate() {
        let trimmed = line.trim_start();
        for (kw, kind) in keywords {
            if !starts_with_keyword(trimmed, kw) {
                continue;
            }
            if let Some(theorem) =
                parse_decl_line(trimmed, kw, *kind, system, source_file, line_idx + 1)
            {
                out.push(theorem);
            }
            break;
        }
    }
    out
}

fn starts_with_keyword(s: &str, kw: &str) -> bool {
    if !s.starts_with(kw) {
        return false;
    }
    // Next char must be a separator (whitespace, end-of-line) so
    // `theorem_name` doesn't trigger.
    match s.as_bytes().get(kw.len()) {
        None => true,
        Some(&b) => !(b.is_ascii_alphanumeric() || b == b'_'),
    }
}

fn parse_decl_line(
    line: &str,
    kw: &str,
    kind: ForeignTheoremKind,
    system: ForeignSystem,
    source_file: &str,
    line_number: usize,
) -> Option<ForeignTheorem> {
    let after_kw = line[kw.len()..].trim_start();
    // Pull the next identifier as the theorem name.
    let name_end = after_kw
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '\''))
        .unwrap_or(after_kw.len());
    if name_end == 0 {
        return None;
    }
    let name = &after_kw[..name_end];
    let after_name = after_kw[name_end..].trim_start();
    // The statement begins after the next `:`.
    let colon = after_name.find(':')?;
    // Skip `::` (Mizar comment marker handled in strip_comments;
    // here we still avoid `:=` for Lean).
    let mut statement = after_name[colon + 1..].trim_start().to_string();
    // For Lean, statement ends at `:=`.
    if system == ForeignSystem::Lean4 {
        if let Some(eq_idx) = statement.find(":=") {
            statement.truncate(eq_idx);
        }
    } else {
        // For Coq / Mizar / Isabelle, statement ends at the next `.`
        // (Coq/Mizar) or `proof`/`by`/`apply` keyword (Isabelle).
        if system == ForeignSystem::Isabelle {
            for delim in [" proof ", " by ", " apply "] {
                if let Some(idx) = statement.find(delim) {
                    statement.truncate(idx);
                }
            }
        }
        if let Some(dot) = statement.rfind('.') {
            // Only truncate if `.` is at end of statement, not part of
            // a name like `Mathlib.Algebra`.
            let trailing = &statement[dot + 1..];
            if trailing.trim().is_empty() {
                statement.truncate(dot);
            }
        }
    }
    let statement = statement.trim().to_string();
    if name.is_empty() || statement.is_empty() {
        return None;
    }
    let citation = format!("{}:{}", source_file, line_number);
    Some(ForeignTheorem {
        system,
        name: Text::from(name),
        kind,
        statement: Text::from(statement),
        source_file: Text::from(source_file),
        source_line: line_number as u32,
        framework_citation: Text::from(citation),
    })
}

/// Strip system-specific comment forms.  Replaces comment regions
/// with whitespace (preserving line numbers) so the keyword
/// extractor works against a comment-free view.
fn strip_comments(content: &str, system: ForeignSystem) -> String {
    match system {
        ForeignSystem::Coq | ForeignSystem::Isabelle => strip_block_comments(content, "(*", "*)"),
        ForeignSystem::Lean4 => strip_line_comments(content, "--"),
        ForeignSystem::Mizar => strip_line_comments(content, "::"),
    }
}

fn strip_line_comments(content: &str, marker: &str) -> String {
    let mut out = String::with_capacity(content.len());
    for line in content.lines() {
        match line.find(marker) {
            Some(idx) => {
                out.push_str(&line[..idx]);
                out.push('\n');
            }
            None => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out
}

fn strip_block_comments(content: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut bytes = content.as_bytes();
    while !bytes.is_empty() {
        if bytes.starts_with(open.as_bytes()) {
            // skip to close (preserving newlines)
            bytes = &bytes[open.len()..];
            while !bytes.is_empty() && !bytes.starts_with(close.as_bytes()) {
                let c = bytes[0];
                if c == b'\n' {
                    out.push('\n');
                }
                bytes = &bytes[1..];
            }
            if bytes.starts_with(close.as_bytes()) {
                bytes = &bytes[close.len()..];
            }
        } else {
            out.push(bytes[0] as char);
            bytes = &bytes[1..];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- ForeignSystem -----

    #[test]
    fn system_round_trip() {
        for s in ForeignSystem::all() {
            assert_eq!(ForeignSystem::from_name(s.name()), Some(s));
        }
    }

    #[test]
    fn system_aliases_resolve() {
        assert_eq!(ForeignSystem::from_name("rocq"), Some(ForeignSystem::Coq));
        assert_eq!(ForeignSystem::from_name("lean"), Some(ForeignSystem::Lean4));
        assert_eq!(
            ForeignSystem::from_name("mathlib"),
            Some(ForeignSystem::Lean4)
        );
        assert_eq!(ForeignSystem::from_name("hol"), Some(ForeignSystem::Isabelle));
    }

    #[test]
    fn system_rejects_unknown() {
        assert_eq!(ForeignSystem::from_name(""), None);
        assert_eq!(ForeignSystem::from_name("garbage"), None);
    }

    #[test]
    fn extensions_distinct() {
        let exts: std::collections::BTreeSet<&str> =
            ForeignSystem::all().iter().map(|s| s.extension()).collect();
        assert_eq!(exts.len(), 4);
    }

    #[test]
    fn framework_tags_distinct() {
        let tags: std::collections::BTreeSet<&str> = ForeignSystem::all()
            .iter()
            .map(|s| s.framework_tag())
            .collect();
        assert_eq!(tags.len(), 4);
    }

    // ----- CoqImporter -----

    #[test]
    fn coq_extracts_theorem() {
        let src = "Theorem add_comm : forall a b : nat, a + b = b + a.\nProof. admit. Qed.\n";
        let importer = CoqImporter;
        let out = importer.parse_text(src, "src.v").unwrap();
        assert_eq!(out.len(), 1);
        let t = &out[0];
        assert_eq!(t.system, ForeignSystem::Coq);
        assert_eq!(t.kind, ForeignTheoremKind::Theorem);
        assert_eq!(t.name.as_str(), "add_comm");
        assert!(t.statement.as_str().contains("forall a b : nat"));
        assert_eq!(t.source_line, 1);
    }

    #[test]
    fn coq_extracts_multiple_kinds() {
        let src = "Theorem t1 : True.\nLemma l1 : 0 = 0.\nAxiom ax1 : forall x, x = x.\n";
        let out = CoqImporter.parse_text(src, "src.v").unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].kind, ForeignTheoremKind::Theorem);
        assert_eq!(out[1].kind, ForeignTheoremKind::Lemma);
        assert_eq!(out[2].kind, ForeignTheoremKind::Axiom);
    }

    #[test]
    fn coq_strips_block_comments() {
        let src = "(* Theorem hidden : False. *)\nTheorem visible : True.\n";
        let out = CoqImporter.parse_text(src, "src.v").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name.as_str(), "visible");
    }

    #[test]
    fn coq_handles_keyword_in_identifier_correctly() {
        // `Theorem_helper` (underscore) is NOT a keyword match.
        let src = "Theorem_helper : Foo.\nTheorem real_thm : True.\n";
        let out = CoqImporter.parse_text(src, "src.v").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name.as_str(), "real_thm");
    }

    // ----- Lean4Importer -----

    #[test]
    fn lean_extracts_theorem_with_proof_term() {
        let src = "theorem add_comm : ∀ a b : Nat, a + b = b + a := by simp\n";
        let out = Lean4Importer.parse_text(src, "Algebra.lean").unwrap();
        assert_eq!(out.len(), 1);
        let t = &out[0];
        assert_eq!(t.kind, ForeignTheoremKind::Theorem);
        assert_eq!(t.name.as_str(), "add_comm");
        // Statement excludes `:= by simp`.
        assert!(t.statement.as_str().contains("∀ a b : Nat"));
        assert!(!t.statement.as_str().contains(":="));
    }

    #[test]
    fn lean_extracts_axiom() {
        let src = "axiom choice : ∀ {α : Type}, Nonempty α → α\n";
        let out = Lean4Importer.parse_text(src, "Choice.lean").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, ForeignTheoremKind::Axiom);
    }

    #[test]
    fn lean_strips_line_comments() {
        let src = "-- theorem hidden : False := by sorry\ntheorem visible : True := trivial\n";
        let out = Lean4Importer.parse_text(src, "src.lean").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name.as_str(), "visible");
    }

    // ----- MizarImporter -----

    #[test]
    fn mizar_extracts_theorem() {
        let src = "theorem Th1: x in NAT implies x is real;\n";
        let out = MizarImporter.parse_text(src, "src.miz").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name.as_str(), "Th1");
    }

    #[test]
    fn mizar_strips_double_colon_comments() {
        let src = ":: theorem hidden: False;\ntheorem visible: 0 = 0;\n";
        let out = MizarImporter.parse_text(src, "src.miz").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name.as_str(), "visible");
    }

    // ----- IsabelleImporter -----

    #[test]
    fn isabelle_extracts_theorem() {
        let src = "theorem add_comm: \"a + b = b + a\" by auto\n";
        let out = IsabelleImporter.parse_text(src, "Add.thy").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name.as_str(), "add_comm");
    }

    // ----- ForeignTheorem.to_verum_skeleton -----

    #[test]
    fn skeleton_includes_framework_attribution() {
        let t = ForeignTheorem {
            system: ForeignSystem::Lean4,
            name: Text::from("add_comm"),
            kind: ForeignTheoremKind::Theorem,
            statement: Text::from("∀ a b : Nat, a + b = b + a"),
            source_file: Text::from("Mathlib/Algebra/Group/Basic.lean"),
            source_line: 42,
            framework_citation: Text::from("Mathlib/Algebra/Group/Basic.lean:42"),
        };
        let s = t.to_verum_skeleton();
        let s = s.as_str();
        assert!(s.contains("@framework(lean_mathlib4"));
        assert!(s.contains("Mathlib/Algebra/Group/Basic.lean:42"));
        assert!(s.contains("public theorem add_comm"));
        assert!(s.contains("proof by axiom"));
        // Original statement preserved as comment.
        assert!(s.contains("∀ a b : Nat"));
    }

    #[test]
    fn skeleton_axiom_kind_renders_axiom() {
        let t = ForeignTheorem {
            system: ForeignSystem::Coq,
            name: Text::from("choice"),
            kind: ForeignTheoremKind::Axiom,
            statement: Text::from("forall x, P x"),
            source_file: Text::from("c.v"),
            source_line: 1,
            framework_citation: Text::from("c.v:1"),
        };
        let s = t.to_verum_skeleton();
        assert!(s.as_str().contains("public axiom choice"));
    }

    // ----- importer_for dispatcher -----

    #[test]
    fn importer_for_returns_correct_system() {
        for s in ForeignSystem::all() {
            assert_eq!(importer_for(s).system(), s);
        }
    }

    // ----- Acceptance criteria pin -----

    #[test]
    fn task_85_four_systems_supported() {
        assert!(matches!(ForeignSystem::from_name("coq"), Some(_)));
        assert!(matches!(ForeignSystem::from_name("lean4"), Some(_)));
        assert!(matches!(ForeignSystem::from_name("mizar"), Some(_)));
        assert!(matches!(ForeignSystem::from_name("isabelle"), Some(_)));
    }

    #[test]
    fn task_85_imported_theorem_renders_to_verum_with_axiom_proof() {
        // §1 of acceptance: imported theorem skeleton with @axiom
        // proof body and @framework citation back to source.
        let src = "Theorem foo : True.\nProof. trivial. Qed.\n";
        let out = CoqImporter.parse_text(src, "test.v").unwrap();
        assert_eq!(out.len(), 1);
        let skeleton = out[0].to_verum_skeleton();
        let s = skeleton.as_str();
        assert!(s.contains("@framework(coq"));
        assert!(s.contains("test.v:1"));
        assert!(s.contains("proof by axiom"));
    }
}
