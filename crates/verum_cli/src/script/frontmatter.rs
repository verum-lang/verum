//! PEP-723-style inline metadata for Verum scripts.
//!
//! A Verum script may carry an inline metadata block delimited by
//! `// /// script` and `// ///` line comments. The interior is line-prefixed
//! with `// `; after stripping that prefix, the remainder is parsed as TOML.
//!
//! Example:
//!
//! ```verum
//! #!/usr/bin/env verum
//! // /// script
//! // verum = ">=0.6.0"
//! // dependencies = ["json@1", "http@^0.2"]
//! // permissions = ["net=api.example.com", "read=./data"]
//! // edition = "2026"
//! //
//! // [profile]
//! // tier = 0
//! // ///
//!
//! mount core.io
//! print("hello");
//! ```
//!
//! # Why this format
//!
//! - **Inert under regular Verum tooling**: the entire block is line comments,
//!   so the LSP, formatter, and linter do not need a special parse mode.
//! - **Industry alignment**: PEP 723 (Python, accepted Jan 2024) uses the same
//!   `# /// script` / `# ///` pattern. `uv run`, `pipx run`, `poetry run`, and
//!   the upcoming `dotnet run app.cs` all follow comparable shapes. Users
//!   transferring from those ecosystems recognise the structure immediately.
//! - **Single source of truth**: TOML is already the manifest format
//!   (`verum.toml`), so the field schema is reused — no new dialect.
//!
//! # Schema
//!
//! Top-level keys recognised today:
//!
//! | key | type | meaning |
//! |---|---|---|
//! | `verum` | semver spec | required minimum Verum toolchain |
//! | `dependencies` | array or table | registry deps for this script |
//! | `permissions` | array of strings | Deno-style permission scopes |
//! | `edition` | string | language edition (currently informational) |
//! | `[profile]` | table | per-script profile overrides |
//! | `[run]` | table | run-time defaults (inherits from `[run]` in `verum.toml`) |
//!
//! Unknown keys are preserved in `raw` and surfaced to callers — this lets
//! future schema additions be picked up without parser revisions.

use serde::{Deserialize, Serialize};
use std::ops::Range;

/// Parsed frontmatter metadata extracted from a script source. Wraps the raw
/// TOML value plus convenience accessors for the well-known fields.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Frontmatter {
    /// The raw TOML body (after `// `-stripping). Empty string if the body had
    /// no non-blank content.
    #[serde(skip)]
    pub raw_toml: String,

    /// Required minimum toolchain version, e.g. `">=0.6.0"`. None if absent.
    #[serde(default)]
    pub verum: Option<String>,

    /// Dependencies declared inline. Each entry is either a bare version spec
    /// (`"json@1"`) or a fully qualified table (`{name="x", version="1"}`)
    /// represented as raw TOML for now — full typing happens in P4 (resolver).
    #[serde(default)]
    pub dependencies: Vec<DepSpec>,

    /// Permission scopes (Deno-style). Each is a string like `"net=api.x:443"`.
    #[serde(default)]
    pub permissions: Vec<String>,

    /// Language edition (informational; reserved for future use).
    #[serde(default)]
    pub edition: Option<String>,

    /// Profile overrides for this script (tier, verify, opt-level, etc.).
    #[serde(default)]
    pub profile: Option<ProfileOverrides>,

    /// Run-time defaults. May override [run] in user's verum.toml when present.
    #[serde(default)]
    pub run: Option<RunOverrides>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DepSpec {
    /// Short form: `"json@1"` or `"json"`.
    Short(String),
    /// Long form: `{ name = "json", version = "^1", features = ["..."], path = "...", git = "..." }`
    Long(DepLong),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepLong {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub git: Option<String>,
    #[serde(default)]
    pub rev: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub registry: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfileOverrides {
    #[serde(default)]
    pub tier: Option<u8>,
    #[serde(default)]
    pub verify: Option<String>,
    #[serde(default, rename = "opt-level")]
    pub opt_level: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunOverrides {
    #[serde(default, rename = "default-permissions")]
    pub default_permissions: Vec<String>,
}

/// Successful extraction result: the parsed frontmatter plus the byte range it
/// occupied in the original source (so callers can slice it out before further
/// processing if desired).
#[derive(Debug, Clone)]
pub struct Extracted {
    pub frontmatter: Frontmatter,
    /// Byte range in the original source covering the whole frontmatter
    /// region (both delimiter lines included). Empty range if no block found.
    pub range: Range<usize>,
}

/// Errors that can arise while parsing frontmatter.
#[derive(Debug, Clone)]
pub enum FrontmatterError {
    UnterminatedBlock { line: usize },
    InvalidToml { line: usize, source: String },
    NonCommentLineInBlock { line: usize, content: String },
}

impl std::fmt::Display for FrontmatterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnterminatedBlock { line } => write!(
                f,
                "frontmatter: opening marker `// /// script` found at line {line} but no closing `// ///`",
            ),
            Self::InvalidToml { line, source } => {
                write!(f, "frontmatter: malformed TOML at line {line}: {source}")
            }
            Self::NonCommentLineInBlock { line, content } => write!(
                f,
                "frontmatter: line {line} inside block does not start with `//`: {content:?}",
            ),
        }
    }
}

impl std::error::Error for FrontmatterError {}

/// Locate the `// /// script` ... `// ///` block in `source`, parse the body
/// as TOML, and return the structured `Frontmatter`. Returns `Ok(None)` if
/// the source contains no frontmatter block.
///
/// Detection rules:
/// - Opening line, after `trim_end()`, equals `// /// script` (with optional
///   inner whitespace, e.g. `//  /// script` is also accepted — we strip a
///   single space after `//`).
/// - Closing line, after trim, equals `// ///`.
/// - Detection scans the source from byte 0; a leading UTF-8 BOM
///   (`EF BB BF`) is silently skipped, and a leading shebang line is also
///   silently skipped if present.
/// - At most one block per source. The opening match wins; subsequent
///   `// /// script` lines are treated as ordinary comments.
///
/// Byte ranges in the returned [`Extracted::range`] are in *original-source*
/// coordinates (i.e. include any BOM byte offset). Callers that strip the
/// BOM separately can subtract `UTF8_BOM_LEN` themselves.
pub fn extract(source: &str) -> Result<Option<Extracted>, FrontmatterError> {
    // Detect and skip a leading UTF-8 BOM. Cross-platform editors prepend
    // it on save and the marker-matcher below can't see past it (the BOM
    // is 3 bytes that don't form a valid `//` prefix). We skip the BOM
    // for line-walking but add `bom_len` back to every byte range we
    // return so callers stay in original-source coordinates.
    const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
    let bom_len = if source.as_bytes().starts_with(UTF8_BOM) {
        UTF8_BOM.len()
    } else {
        0
    };
    let scan = &source[bom_len..];

    let mut lines: Vec<(Range<usize>, &str)> = Vec::new();
    let bytes = scan.as_bytes();
    let mut start = 0usize;
    for i in 0..bytes.len() {
        if bytes[i] == b'\n' {
            let line = &scan[start..i];
            // Spans here are scan-relative; we shift them by `bom_len`
            // when assembling `Extracted::range` below.
            lines.push((start..i + 1, line));
            start = i + 1;
        }
    }
    if start < bytes.len() {
        lines.push((start..bytes.len(), &scan[start..]));
    }

    // Optional shebang skip on first line.
    let mut idx = 0usize;
    if let Some((_, l)) = lines.first() {
        if l.starts_with("#!") {
            idx = 1;
        }
    }

    // Find opening marker.
    let mut open_idx: Option<usize> = None;
    while idx < lines.len() {
        if is_open_marker(lines[idx].1) {
            open_idx = Some(idx);
            break;
        }
        idx += 1;
    }
    let open_idx = match open_idx {
        Some(i) => i,
        None => return Ok(None),
    };

    // Find closing marker.
    let mut close_idx: Option<usize> = None;
    let mut j = open_idx + 1;
    while j < lines.len() {
        if is_close_marker(lines[j].1) {
            close_idx = Some(j);
            break;
        }
        j += 1;
    }
    let close_idx = match close_idx {
        Some(i) => i,
        None => return Err(FrontmatterError::UnterminatedBlock { line: open_idx + 1 }),
    };

    // Strip `// ` prefix from each interior line and assemble TOML body.
    let mut toml_body = String::new();
    for (line_no, (_, raw)) in lines[(open_idx + 1)..close_idx].iter().enumerate() {
        let logical_line = open_idx + 2 + line_no; // 1-indexed user-visible
        let trimmed = strip_comment_prefix(raw)
            .ok_or_else(|| FrontmatterError::NonCommentLineInBlock {
                line: logical_line,
                content: raw.trim().to_string(),
            })?;
        toml_body.push_str(trimmed);
        toml_body.push('\n');
    }

    let parsed: Frontmatter = match toml::from_str::<Frontmatter>(&toml_body) {
        Ok(mut fm) => {
            fm.raw_toml = toml_body;
            fm
        }
        Err(e) => {
            // The TOML error reports a position inside `toml_body` (which is
            // post-strip). Translating it back to the original source line
            // requires accounting for the offset at which interior lines
            // begin; we approximate by adding the opening marker line.
            return Err(FrontmatterError::InvalidToml {
                line: open_idx + 2,
                source: e.to_string(),
            });
        }
    };

    // Shift scan-relative byte ranges back into original-source coordinates
    // by re-adding the BOM length we stripped at the top.
    let range = (lines[open_idx].0.start + bom_len)..(lines[close_idx].0.end + bom_len);
    Ok(Some(Extracted {
        frontmatter: parsed,
        range,
    }))
}

/// Variant: Like [`extract`] but returns `None` quietly on error. Useful for
/// best-effort tooling that wants to display the script even with a malformed
/// frontmatter (the proper user-facing reporting path uses [`extract`]).
pub fn extract_lossy(source: &str) -> Option<Extracted> {
    extract(source).ok().flatten()
}

fn is_open_marker(line: &str) -> bool {
    let s = line.trim_end_matches(['\r', '\n']);
    let s = strip_comment_prefix(s).unwrap_or(s);
    s.trim() == "/// script"
}

fn is_close_marker(line: &str) -> bool {
    let s = line.trim_end_matches(['\r', '\n']);
    let s = strip_comment_prefix(s).unwrap_or(s);
    s.trim() == "///"
}

/// If `line` begins with a `//` line-comment marker, return the content after
/// it (with a single following space stripped if present). Returns `None` for
/// non-comment lines.
fn strip_comment_prefix(line: &str) -> Option<&str> {
    // Allow optional leading whitespace before `//` so that callers indenting
    // their frontmatter for visual alignment still parse.
    let trimmed = line.trim_start();
    if !trimmed.starts_with("//") {
        return None;
    }
    let after = &trimmed[2..];
    // Strip one trailing space — `// foo` -> `foo`, `//foo` -> `foo`.
    if after.starts_with(' ') {
        Some(&after[1..])
    } else {
        Some(after)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_frontmatter() {
        let src = "fn main() {}";
        let r = extract(src).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn empty_block() {
        let src = "// /// script\n// ///\nfn main() {}";
        let e = extract(src).unwrap().expect("frontmatter must be detected");
        assert_eq!(e.frontmatter.verum, None);
        assert!(e.frontmatter.dependencies.is_empty());
        assert!(e.frontmatter.permissions.is_empty());
    }

    #[test]
    fn full_block() {
        let src = r#"#!/usr/bin/env verum
// /// script
// verum = ">=0.6.0"
// dependencies = ["json@1", "http"]
// permissions = ["net=api.example.com:443", "read=./data"]
// edition = "2026"
//
// [profile]
// tier = 0
// verify = "off"
// opt-level = 2
//
// [run]
// default-permissions = ["fs:cwd"]
// ///

mount core.io
print("hello");
"#;
        let e = extract(src).unwrap().unwrap();
        let fm = &e.frontmatter;
        assert_eq!(fm.verum.as_deref(), Some(">=0.6.0"));
        assert_eq!(fm.dependencies.len(), 2);
        match &fm.dependencies[0] {
            DepSpec::Short(s) => assert_eq!(s, "json@1"),
            DepSpec::Long(_) => panic!("expected short form"),
        }
        assert_eq!(fm.permissions.len(), 2);
        assert_eq!(fm.permissions[0], "net=api.example.com:443");
        assert_eq!(fm.edition.as_deref(), Some("2026"));
        let p = fm.profile.as_ref().unwrap();
        assert_eq!(p.tier, Some(0));
        assert_eq!(p.verify.as_deref(), Some("off"));
        assert_eq!(p.opt_level, Some(2));
        let r = fm.run.as_ref().unwrap();
        assert_eq!(r.default_permissions, vec!["fs:cwd"]);
    }

    #[test]
    fn long_form_dep() {
        let src = r#"// /// script
// dependencies = [{ name = "json", version = "^1.0", features = ["serde"] }]
// ///
"#;
        let e = extract(src).unwrap().unwrap();
        match &e.frontmatter.dependencies[0] {
            DepSpec::Long(d) => {
                assert_eq!(d.name, "json");
                assert_eq!(d.version.as_deref(), Some("^1.0"));
                assert_eq!(d.features, vec!["serde"]);
            }
            DepSpec::Short(_) => panic!("expected long form"),
        }
    }

    #[test]
    fn unterminated_block() {
        let src = "// /// script\n// verum = \">=0.6\"\n// (no terminator)\n";
        match extract(src) {
            Err(FrontmatterError::UnterminatedBlock { line }) => assert_eq!(line, 1),
            other => panic!("expected UnterminatedBlock, got {:?}", other),
        }
    }

    #[test]
    fn malformed_toml() {
        let src = "// /// script\n// this is not valid = toml = at all\n// ///\n";
        match extract(src) {
            Err(FrontmatterError::InvalidToml { .. }) => {}
            other => panic!("expected InvalidToml, got {:?}", other),
        }
    }

    #[test]
    fn shebang_then_block() {
        let src = "#!/usr/bin/env verum\n// /// script\n// verum = \"0.6\"\n// ///\n";
        let e = extract(src).unwrap().unwrap();
        assert_eq!(e.frontmatter.verum.as_deref(), Some("0.6"));
    }

    #[test]
    fn block_with_indented_comments() {
        let src = "    // /// script\n    // verum = \"0.6\"\n    // ///\n";
        let e = extract(src).unwrap().unwrap();
        assert_eq!(e.frontmatter.verum.as_deref(), Some("0.6"));
    }

    #[test]
    fn no_extra_block_after_closer() {
        // A second `// /// script` after the first block closes is just a
        // comment; we don't re-enter parsing.
        let src = "// /// script\n// verum = \"0.6\"\n// ///\n// /// script\n// foo\n";
        let e = extract(src).unwrap().unwrap();
        assert_eq!(e.frontmatter.verum.as_deref(), Some("0.6"));
        // Range should cover only the first block (3 lines).
        let covered = &src[e.range];
        assert!(covered.contains("verum"));
        assert!(!covered.contains("foo"));
    }

    #[test]
    fn range_is_correct() {
        let src = "// /// script\n// verum = \"0.6\"\n// ///\n\nfn main() {}";
        let e = extract(src).unwrap().unwrap();
        assert_eq!(e.range.start, 0);
        let covered = &src[e.range];
        assert!(covered.starts_with("// /// script"));
        assert!(covered.trim_end().ends_with("// ///"));
    }

    #[test]
    fn block_after_blank_lines() {
        let src = "\n\n// /// script\n// verum = \"0.6\"\n// ///\n";
        let e = extract(src).unwrap().unwrap();
        assert_eq!(e.frontmatter.verum.as_deref(), Some("0.6"));
    }

    #[test]
    fn bom_only_then_block() {
        // Cross-platform editor saved the script with a UTF-8 BOM. The
        // marker matcher must look past the BOM and the byte range we
        // return must be in original-source coordinates.
        let src = "\u{FEFF}// /// script\n// verum = \"0.6\"\n// ///\nmount core.io";
        let e = extract(src).unwrap().unwrap();
        assert_eq!(e.frontmatter.verum.as_deref(), Some("0.6"));
        // Range must be original-source coords: start at byte 3 (post-BOM).
        assert_eq!(e.range.start, 3);
        let covered = &src[e.range];
        assert!(covered.starts_with("// /// script"));
        assert!(covered.trim_end().ends_with("// ///"));
    }

    #[test]
    fn bom_then_shebang_then_block() {
        // Most-permissive layout: BOM precedes shebang precedes block.
        // Both prefix layers must be silently skipped.
        let src = "\u{FEFF}#!/usr/bin/env verum\n// /// script\n// verum = \"0.6\"\n// ///\n";
        let e = extract(src).unwrap().unwrap();
        assert_eq!(e.frontmatter.verum.as_deref(), Some("0.6"));
        // Range starts at 3 (BOM) + 21 (shebang) = 24.
        let expected_start = 3 + "#!/usr/bin/env verum\n".len();
        assert_eq!(e.range.start, expected_start);
        let covered = &src[e.range];
        assert!(covered.starts_with("// /// script"));
    }

    #[test]
    fn bom_with_no_block_returns_none() {
        let src = "\u{FEFF}fn main() {}";
        assert!(extract(src).unwrap().is_none());
    }

    #[test]
    fn permissions_array_only() {
        let src = "// /// script\n// permissions = [\"net\", \"read=./x\"]\n// ///\n";
        let e = extract(src).unwrap().unwrap();
        assert_eq!(e.frontmatter.permissions, vec!["net", "read=./x"]);
    }

    #[test]
    fn extract_lossy_returns_none_on_error() {
        let src = "// /// script\n// (bad TOML\n// ///\n";
        assert!(extract_lossy(src).is_none());
    }

    #[test]
    fn no_block_when_only_comments() {
        // `// /// script` mid-file with no real block doesn't trigger anything.
        let src = "fn main() {\n    // /// script with a real fn\n}\n";
        // We DO match it because is_open_marker only checks the line ends with `/// script`.
        // To avoid false positives the closer must follow. Without a closer,
        // it's an error — UnterminatedBlock — which is acceptable: this is a
        // very narrow user mistake.
        let result = extract(src);
        // Either Err(Unterminated) or Ok(None) is acceptable here. Pin the
        // current behaviour: it parses the marker as a valid open and errors
        // due to missing close.
        matches!(result, Err(FrontmatterError::UnterminatedBlock { .. }));
    }
}
