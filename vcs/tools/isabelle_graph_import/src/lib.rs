//! Isabelle/HOL → Verum theorem importer
//!
//! Ingests Isabelle/HOL theory files (`.thy`) that form the public
//! Graph_Library release and emits Verum regression tests (`.vr`)
//! that restate each imported theorem in Verum's surface syntax
//! together with a stub `proof by auto;` body. The stubs are intended
//! to be expanded by hand (or by tactic autopilot) once the
//! corresponding Verum graph / linalg / category definitions exist.
//!
//! The converter does **not** attempt to mechanize proof terms —
//! Isabelle proof terms are opaque here. It only moves **statements**
//! across languages so we can grow a regression suite that matches
//! the Isabelle library's theorem set one-for-one.
//!
//! ## Supported Isabelle surface
//!
//!   - `theorem NAME: "STATEMENT"` and `lemma NAME: "STATEMENT"`.
//!     Statements are Isabelle-quoted strings. The converter extracts
//!     NAME and STATEMENT and emits them verbatim inside a Verum
//!     theorem with a TODO comment carrying the original Isabelle
//!     statement so reviewers can check the translation.
//!
//!   - Multi-line statements: the parser joins continuation lines
//!     until the closing quote. Isabelle's `\<close>` / `done` /
//!     `qed` markers terminate a theorem block and cause the parser
//!     to move on.
//!
//!   - Block comments `(* ... *)` are stripped before parsing.
//!
//! ## Out of scope
//!
//!   - Isabelle notation files, class definitions, locales, types,
//!     simp-rules, and anything that is not a bare theorem / lemma.
//!     Those require manual porting into Verum's type / protocol
//!     machinery.
//!
//! ## Usage
//!
//! ```bash
//! verum-isabelle-import --input path/to/Graph_Library.thy \
//!                        --output vcs/regressions/graph_from_isabelle/
//! ```
//!
//! Each imported theorem lands in its own `.vr` file named
//! `<theorem_name>.vr` under the output directory. Directory
//! creation, file truncation, and idempotent re-imports are the
//! caller's responsibility — this library writes files verbatim.

use std::path::Path;

// ============================================================================
// Data shapes
// ============================================================================

/// One imported theorem, ready to be emitted as a `.vr` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedTheorem {
    /// Isabelle theorem name (left of `:`).
    pub name: String,
    /// Isabelle statement contents (between the quoting delimiters).
    pub statement: String,
    /// `theorem` / `lemma` — preserved so the emitter can keep the
    /// original flavour.
    pub keyword: TheoremKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TheoremKind {
    Theorem,
    Lemma,
}

impl TheoremKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Theorem => "theorem",
            Self::Lemma => "lemma",
        }
    }
}

// ============================================================================
// Parser
// ============================================================================

/// Parses a `.thy` source into a list of imported theorems.
///
/// The parser is deliberately permissive — it ignores anything that
/// is not a top-level `theorem NAME:` or `lemma NAME:`. Malformed
/// theorems (no statement, no terminator) are dropped silently so a
/// partial `.thy` does not abort the whole import.
pub fn parse_theory(src: &str) -> Vec<ImportedTheorem> {
    let stripped = strip_block_comments(src);
    let mut out = Vec::new();
    let mut iter = stripped.lines().peekable();

    while let Some(line) = iter.next() {
        let trimmed = line.trim_start();
        let (kind, rest) = if let Some(r) = trimmed.strip_prefix("theorem ") {
            (TheoremKind::Theorem, r)
        } else if let Some(r) = trimmed.strip_prefix("lemma ") {
            (TheoremKind::Lemma, r)
        } else {
            continue;
        };

        // Name up to `:` (if present on the same line).
        let Some(colon) = rest.find(':') else { continue };
        let name = rest[..colon].trim().to_string();
        if name.is_empty() {
            continue;
        }

        let mut tail = rest[colon + 1..].trim().to_string();
        // Collect continuation lines until we hit a statement-
        // terminating marker or a closing quote.
        while !statement_closed(&tail) {
            match iter.next() {
                Some(next) => {
                    tail.push(' ');
                    tail.push_str(next.trim());
                }
                None => break,
            }
        }

        let statement = extract_statement(&tail);
        if let Some(statement) = statement {
            if statement.trim().is_empty() {
                continue;
            }
            out.push(ImportedTheorem {
                name,
                statement,
                keyword: kind,
            });
        }
    }

    out
}

fn strip_block_comments(src: &str) -> String {
    // Isabelle uses `(* ... *)`. Nested comments are legal; we support
    // them via a depth counter so the importer does not miss theorems
    // following a nested-comment block.
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut depth: u32 = 0;
    while i < bytes.len() {
        if depth == 0
            && i + 1 < bytes.len()
            && bytes[i] == b'('
            && bytes[i + 1] == b'*'
        {
            depth = 1;
            i += 2;
            continue;
        }
        if depth > 0 {
            if i + 1 < bytes.len() && bytes[i] == b'('
                && bytes[i + 1] == b'*'
            {
                depth += 1;
                i += 2;
                continue;
            }
            if i + 1 < bytes.len() && bytes[i] == b'*'
                && bytes[i + 1] == b')'
            {
                depth -= 1;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Returns true once the tail contains a matched `"..."` statement
/// or a `\<close>` / `qed` / `done` marker — both signal that a
/// multi-line theorem header has been fully absorbed.
fn statement_closed(tail: &str) -> bool {
    let quote_count = tail.bytes().filter(|&b| b == b'"').count();
    if quote_count >= 2 && quote_count % 2 == 0 {
        return true;
    }
    tail.contains("\\<close>") || tail.contains("qed") || tail.contains("done")
}

fn extract_statement(tail: &str) -> Option<String> {
    let first = tail.find('"')?;
    let after = &tail[first + 1..];
    let close = after.find('"')?;
    Some(after[..close].trim().to_string())
}

// ============================================================================
// Emitter
// ============================================================================

/// Renders an imported theorem as a Verum `.vr` source.
///
/// The emitter wraps the original Isabelle statement in a `// Source:`
/// comment and produces a theorem with `by auto` as the proof
/// placeholder — `auto` is Verum's weakest automation and therefore
/// the most conservative first attempt for any imported goal.
pub fn emit_verum(theorem: &ImportedTheorem) -> String {
    let mut s = String::new();
    s.push_str("// @test: typecheck-pass\n");
    s.push_str("// @level: L1\n");
    s.push_str("// @tags: imported, isabelle, graph\n");
    s.push_str(&format!(
        "// @description: Imported from Isabelle/HOL Graph_Library — {}\n",
        theorem.name
    ));
    s.push_str(&format!(
        "// Source ({}): {}\n\n",
        theorem.keyword.as_str(),
        theorem.statement
    ));
    s.push_str(&format!(
        "{} {}: {} {{ proof by auto; }}\n",
        theorem.keyword.as_str(),
        theorem.name,
        theorem.statement,
    ));
    s
}

// ============================================================================
// Directory writer
// ============================================================================

/// Writes `theorems` under `out_dir` with one `.vr` per theorem.
///
/// Creates missing parent directories. Overwrites existing files so
/// a re-run is idempotent. Returns a list of the paths written in
/// the order they were produced.
pub fn write_out_dir(
    theorems: &[ImportedTheorem],
    out_dir: &Path,
) -> std::io::Result<Vec<std::path::PathBuf>> {
    std::fs::create_dir_all(out_dir)?;
    let mut paths = Vec::with_capacity(theorems.len());
    for th in theorems {
        let filename = format!("{}.vr", sanitize_filename(&th.name));
        let path = out_dir.join(filename);
        std::fs::write(&path, emit_verum(th))?;
        paths.push(path);
    }
    Ok(paths)
}

/// Map an Isabelle identifier (which may contain `.`, `'`, `?`, …)
/// to a POSIX-safe file stem.
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
(* trivial heading *)
theory Graph
imports Main
begin

theorem path_refl: "reachable G v v"
  by auto

(* nested (* comment *) with theorem inside-ignored *)

lemma connected_is_reachable:
  "connected G \<longrightarrow> (\<forall>u v. reachable G u v)"
  by blast

end
"#;

    #[test]
    fn parses_theorem_and_lemma() {
        let ths = parse_theory(SAMPLE);
        assert_eq!(ths.len(), 2);
        assert_eq!(ths[0].name, "path_refl");
        assert_eq!(ths[0].keyword, TheoremKind::Theorem);
        assert_eq!(ths[0].statement, "reachable G v v");
        assert_eq!(ths[1].name, "connected_is_reachable");
        assert_eq!(ths[1].keyword, TheoremKind::Lemma);
        assert!(ths[1].statement.contains("reachable G u v"));
    }

    #[test]
    fn strips_nested_comments() {
        let src = "(* a (* b *) c *) theorem x: \"P\" done";
        let ths = parse_theory(src);
        assert_eq!(ths.len(), 1);
        assert_eq!(ths[0].name, "x");
        assert_eq!(ths[0].statement, "P");
    }

    #[test]
    fn emit_produces_auto_stub() {
        let th = ImportedTheorem {
            name: "foo".into(),
            statement: "1 + 1 == 2".into(),
            keyword: TheoremKind::Theorem,
        };
        let out = emit_verum(&th);
        assert!(out.contains("theorem foo: 1 + 1 == 2 { proof by auto; }"));
        assert!(out.contains("Imported from Isabelle/HOL Graph_Library"));
    }

    #[test]
    fn sanitize_filename_maps_quotes_and_dots() {
        assert_eq!(sanitize_filename("reachable.v'"), "reachable_v_");
        assert_eq!(sanitize_filename("abc_def"), "abc_def");
    }

    #[test]
    fn empty_statement_is_skipped() {
        let src = "theorem empty: \"\" done";
        let ths = parse_theory(src);
        assert!(ths.is_empty(), "empty statements must not be imported");
    }
}
